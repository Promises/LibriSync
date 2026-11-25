//! The plan executor behind [`super::run`].
//!
//! Port of `enqueuePlan`, `triggerConversion`, `triggerPlainAudioCopy`, `joinParts`,
//! `encodeChaptersToMp3` and `copyToFinalDestination` in
//! `modules/expo-rust-bridge/android/src/main/java/expo/modules/rustbridge/DownloadOrchestrator.kt`,
//! and `validateAudioFile` in `AudioValidation.kt`.
//!
//! Stage order per plan shape:
//! - AAXC / AAX: download → decrypt (+tags, cover) → validate → M4B, or MP3 per chapter
//! - one plain file: download → validate → copy (or retag) → M4B/MP3, or split to MP3
//! - `join_parts`: download all → join (+chapters, tags, cover) → validate → as above
//! - ZIP (and multi-part plain): download all → extract/copy as-is
//!
//! Deliberate differences from Android:
//! - Sources are kept in `work_dir` until the whole run succeeds (Android deleted joined
//!   parts right after the join and the encrypted file on validation failure), so a
//!   failed later step retries without downloading again. A source that was *reused*
//!   from an earlier run and still fails validation is deleted, so the following retry
//!   downloads it fresh instead of failing forever.
//! - Free space is checked on the filesystem each step writes to, before every copy,
//!   decrypt, join, encode and extraction (Android: decrypt/join only, cache volume only).
//! - Validation also fails an MP4 whose decode ends well short of its stated duration
//!   (a truncated download with a front-loaded `moov` decodes without many errors).
//! - Plain single files are validated too, and a plain M4B/M4A is split into MP3s when
//!   MP3 output is chosen (Android silently kept the M4B).
//! - Joined parts with differing codecs/parameters are re-encoded to AAC with the
//!   `concat` filter (Android's stream-copy concat would fail on them).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::providers::{DownloadPart, DownloadPlan};

use super::args::{
    self, drm_input_args, AudioCodec, ChapterSource, Container, Cover, Drm, Input, MetaSource,
    OutputSpec,
};
use super::chapters::{self, Chapter};
use super::download::{fetch_part, Fetched, PartRequest};
use super::fsutil::{
    file_size, io_error, link_or_copy, remove_stale_outputs, remove_stale_temps,
    require_free_space, sanitize_file_name, Staging,
};
use super::progress::Reporter;
use super::sidecars::{self, CoverImage};
use super::tools::{self, cancelled, ffmpeg_failure, probe, run_ffmpeg, FfmpegJob, ProbeInfo};
use super::{
    extract, FailureKind, MediaTools, OutputFormat, PipelineError, PipelineJob, PipelineOutput,
    ProgressFn, Stage, WORK_PREFIX,
};

/// A downloaded part in the work dir.
#[derive(Debug, Clone)]
struct Source {
    path: PathBuf,
    /// Found complete from an earlier run rather than downloaded now.
    reused: bool,
}

/// What the audio step produced (before commit).
struct Produced {
    chapter_count: usize,
    duration_ms: Option<u64>,
}

/// Reject plans the engine can't execute.
pub(crate) fn check_plan(plan: &DownloadPlan) -> Result<(), PipelineError> {
    if plan.parts.is_empty() {
        return Err(PipelineError::new(FailureKind::InvalidPlan, "The download plan has no parts"));
    }
    if plan.join_parts && plan.parts.iter().any(|p| !matches!(p, DownloadPart::Plain { .. })) {
        return Err(PipelineError::new(
            FailureKind::InvalidPlan,
            "A plan that joins its parts may only contain plain files",
        ));
    }
    if plan.parts.len() > 1
        && plan
            .parts
            .iter()
            .any(|p| matches!(p, DownloadPart::Aaxc { .. } | DownloadPart::Aax { .. }))
    {
        return Err(PipelineError::new(
            FailureKind::InvalidPlan,
            "Encrypted (AAXC/AAX) parts are only supported as a single-part plan",
        ));
    }
    Ok(())
}

/// Work-dir file names for each part: the plan's own file name (sanitised, directory
/// stripped) or `partNNN.zip`; prefixed with the part number when names would collide.
/// Deterministic, so a re-planned run finds the previous run's files.
pub(crate) fn work_file_names(plan: &DownloadPlan) -> Vec<String> {
    let base: Vec<String> = plan
        .parts
        .iter()
        .enumerate()
        .map(|(i, part)| {
            let fallback = format!("part{:03}", i + 1);
            match part {
                DownloadPart::Plain { filename, .. }
                | DownloadPart::Aaxc { filename, .. }
                | DownloadPart::Aax { filename, .. } => {
                    let leaf = Path::new(filename)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_default();
                    let name = sanitize_file_name(leaf, &fallback);
                    if name.starts_with(WORK_PREFIX) || name.starts_with('.') {
                        format!("{fallback}-{}", name.trim_start_matches('.'))
                    } else {
                        name
                    }
                }
                DownloadPart::Zip { .. } => format!("{fallback}.zip"),
            }
        })
        .collect();
    let mut seen = HashSet::new();
    let unique = base.iter().all(|n| seen.insert(n.as_str()));
    if unique {
        base
    } else {
        base.into_iter()
            .enumerate()
            .map(|(i, n)| format!("{:03}-{n}", i + 1))
            .collect()
    }
}

fn part_url_headers(part: &DownloadPart) -> (&str, &crate::providers::Headers) {
    match part {
        DownloadPart::Plain { url, headers, .. }
        | DownloadPart::Aaxc { url, headers, .. }
        | DownloadPart::Aax { url, headers, .. }
        | DownloadPart::Zip { url, headers } => (url, headers),
    }
}

struct Executor<'a> {
    job: &'a PipelineJob,
    tools: &'a MediaTools,
    http: &'a reqwest::Client,
    reporter: Arc<Reporter>,
    cancel: &'a CancellationToken,
    /// Audio outputs, committed last.
    audio: Staging,
    /// Sidecars, committed before the audio so a scanner sees them with the book.
    sidecars: Staging,
    /// Intermediate files in the work dir, removed on failure.
    scratch: Vec<PathBuf>,
    file_stem: String,
    cover: Option<CoverImage>,
    /// Write book tags and cover (`embed_metadata`, or always for joins).
    apply_tags: bool,
}

/// Run one job end to end.
pub(crate) async fn execute(
    job: PipelineJob,
    tools: &MediaTools,
    http: &reqwest::Client,
    on_progress: ProgressFn,
    cancel: CancellationToken,
) -> Result<PipelineOutput, PipelineError> {
    check_plan(&job.plan)?;
    for dir in [&job.work_dir, &job.dest_dir] {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| io_error(format_args!("creating {}", dir.display()), e))?;
    }
    remove_stale_temps(&job.dest_dir).await;

    let title_stem = sanitize_file_name(&job.meta.title, &job.meta.product_id);
    let mut exec = Executor {
        job: &job,
        tools,
        http,
        reporter: Arc::new(Reporter::new(on_progress, job.plan.parts.len())),
        cancel: &cancel,
        audio: Staging::default(),
        sidecars: Staging::default(),
        scratch: Vec::new(),
        file_stem: sanitize_file_name(&job.file_stem, &title_stem),
        cover: None,
        apply_tags: job.plan.embed_metadata || job.plan.join_parts,
    };
    log::info!(
        "pipeline: {} \"{}\": {} part(s), format {:?}",
        job.meta.product_id,
        job.meta.title,
        job.plan.parts.len(),
        job.format
    );

    match exec.run().await {
        Ok(output) => {
            if let Err(e) = tokio::fs::remove_dir_all(&job.work_dir).await {
                log::warn!("pipeline: could not remove {}: {e}", job.work_dir.display());
            }
            log::info!(
                "pipeline: {} done: {} file(s), {} bytes",
                job.meta.product_id,
                output.audio_files.len(),
                output.total_bytes
            );
            Ok(output)
        }
        Err(e) => {
            exec.audio.discard().await;
            exec.sidecars.discard().await;
            for path in &exec.scratch {
                let _ = tokio::fs::remove_file(path).await;
            }
            if e.kind == FailureKind::Cancelled {
                log::info!("pipeline: {} cancelled", job.meta.product_id);
            } else {
                log::warn!("pipeline: {} failed ({:?}): {}", job.meta.product_id, e.kind, e.message);
            }
            Err(e)
        }
    }
}

impl Executor<'_> {
    fn plan(&self) -> &DownloadPlan {
        &self.job.plan
    }

    fn work(&self, name: &str) -> PathBuf {
        self.job.work_dir.join(format!("{WORK_PREFIX}{name}"))
    }

    fn check_cancel(&self) -> Result<(), PipelineError> {
        if self.cancel.is_cancelled() {
            Err(cancelled())
        } else {
            Ok(())
        }
    }

    fn cover_arg(&self) -> Option<Cover<'_>> {
        self.cover.as_ref().map(|c| Cover { path: &c.path, copy: c.can_copy() })
    }

    async fn run(&mut self) -> Result<PipelineOutput, PipelineError> {
        let sources = self.download_all().await?;
        self.check_cancel()?;

        let collection = !self.plan().join_parts
            && (self.plan().parts.len() > 1
                || matches!(self.plan().parts[0], DownloadPart::Zip { .. }));
        if let Some(url) = self.job.meta.cover_url.as_deref().filter(|u| !u.trim().is_empty()) {
            if self.job.write_sidecars || !collection {
                self.cover =
                    sidecars::fetch_cover(self.http, url, &self.job.work_dir, self.cancel).await;
                if let Some(cover) = &self.cover {
                    self.scratch.push(cover.path.clone());
                }
            }
        }
        self.check_cancel()?;

        let produced = if self.plan().join_parts {
            self.join(&sources).await?
        } else if collection {
            self.collection(&sources).await?
        } else {
            let source = &sources[0];
            let job = self.job;
            match &job.plan.parts[0] {
                DownloadPart::Aaxc { key, iv, .. } => {
                    self.decrypt(source, Drm::Aaxc { key, iv }).await?
                }
                DownloadPart::Aax { activation_bytes, .. } => {
                    self.decrypt(source, Drm::Aax { activation_bytes }).await?
                }
                DownloadPart::Plain { .. } => self.plain_single(source).await?,
                DownloadPart::Zip { .. } => unreachable!("a single ZIP part is a collection"),
            }
        };
        self.check_cancel()?;
        self.finish(produced).await
    }

    // ---------------------------------------------------------------- download

    async fn download_all(&mut self) -> Result<Vec<Source>, PipelineError> {
        let names = work_file_names(self.plan());
        let mut sources = Vec::with_capacity(names.len());
        for (i, (part, name)) in self.plan().parts.iter().zip(&names).enumerate() {
            let target = self.job.work_dir.join(name);
            let (url, headers) = part_url_headers(part);
            log::info!("pipeline: part {}/{} → {}", i + 1, names.len(), target.display());
            let fetched = fetch_part(
                self.http,
                &PartRequest { url, headers, target: &target, part: i + 1 },
                &self.reporter,
                self.cancel,
            )
            .await?;
            sources.push(Source { path: target, reused: fetched == Fetched::Reused });
        }
        Ok(sources)
    }

    /// A validation-type failure of a source that came from an earlier run: delete it so
    /// the next attempt downloads it again. Fresh downloads are kept (the caller decides).
    async fn on_source_failure(&self, source: &Source, error: PipelineError) -> PipelineError {
        if source.reused && error.kind == FailureKind::Validation {
            log::warn!(
                "pipeline: {} failed again after an earlier run; deleting it so the next attempt downloads it fresh",
                source.path.display()
            );
            let _ = tokio::fs::remove_file(&source.path).await;
        }
        error
    }

    // ---------------------------------------------------------------- validation

    /// Port of `validateAudioFile`: the file must have a duration and decode end to end
    /// without errors.
    async fn validate(&self, path: &Path) -> Result<ProbeInfo, PipelineError> {
        self.reporter.stage(Stage::Validating, Some(0.0));
        let info = probe(self.tools, path, &[], self.cancel)
            .await
            .map_err(|e| e.into_error(FailureKind::Validation, path))?;
        let duration = info.duration_secs.filter(|d| *d > 0.0).ok_or_else(|| {
            PipelineError::new(
                FailureKind::Validation,
                format!("Could not determine the duration of {}", path.display()),
            )
        })?;
        let mut args: Vec<std::ffi::OsString> = vec!["-i".into(), path.as_os_str().to_owned()];
        args.extend(["-map", "0:a:0", "-f", "null", "-"].map(Into::into));
        let outcome = run_ffmpeg(
            self.tools,
            FfmpegJob {
                args,
                stage: Stage::Validating,
                duration_secs: Some(duration),
                span: (0.0, 1.0),
                count_decode_errors: true,
            },
            &self.reporter,
            self.cancel,
        )
        .await?;
        if !outcome.success || outcome.decode_errors > 0 {
            let detail = outcome.stderr_tail.lines().last().unwrap_or("decoder failed").to_string();
            return Err(PipelineError::new(
                FailureKind::Validation,
                format!(
                    "Audio validation failed for {}: {} decode error(s) ({detail})",
                    tools::name_of(path).to_string_lossy(),
                    outcome.decode_errors
                ),
            ));
        }
        // MP4 durations come from the moov box and are exact, so a decode that stops
        // well short means missing media data. (MP3 durations can be estimates.)
        if info.is_mp4() {
            if let Some(decoded) = outcome.out_time_secs {
                let tolerance = (duration * 0.02).max(2.0);
                if decoded + tolerance < duration {
                    return Err(PipelineError::new(
                        FailureKind::Validation,
                        format!(
                            "Audio validation failed for {}: decoded {decoded:.1}s of {duration:.1}s (truncated)",
                            tools::name_of(path).to_string_lossy()
                        ),
                    ));
                }
            }
        }
        self.reporter.stage(Stage::Validating, Some(1.0));
        Ok(info)
    }

    // ---------------------------------------------------------------- decrypt

    async fn decrypt(&mut self, source: &Source, drm: Drm<'_>) -> Result<Produced, PipelineError> {
        let pre = drm_input_args(drm);
        let info = match probe(self.tools, &source.path, &pre, self.cancel).await {
            Ok(info) => info,
            Err(e) => {
                let err = e.into_error(FailureKind::Validation, &source.path);
                return Err(self.on_source_failure(source, err).await);
            }
        };
        let plan_chapters = chapters::from_plan(&self.plan().chapters);
        let inject = if info.chapters.is_empty() && !plan_chapters.is_empty() {
            Some(self.write_chapter_file(&plan_chapters).await?)
        } else {
            None
        };

        let target = self.intermediate_or_final(Container::M4b);
        let size = file_size(&source.path).await;
        require_free_space(parent_of(&target), size)?;
        self.reporter.stage(Stage::Decrypting, Some(0.0));
        log::info!("pipeline: decrypting {}", source.path.display());

        let spec = OutputSpec {
            metadata: MetaSource::KeepSource,
            chapters: inject.as_deref().map_or(ChapterSource::KeepSource, ChapterSource::File),
            cover: self.cover_arg(),
            keep_source_cover: true,
            tags: if self.apply_tags { args::m4b_tags(&self.job.meta) } else { Vec::new() },
            audio: AudioCodec::Copy,
            container: Container::M4b,
            segment: None,
        };
        let args = args::ffmpeg_args(&Input::Single { pre, path: &source.path }, &spec, &target);
        let outcome = run_ffmpeg(
            self.tools,
            FfmpegJob {
                args,
                stage: Stage::Decrypting,
                duration_secs: info.duration_secs,
                span: (0.0, 1.0),
                count_decode_errors: false,
            },
            &self.reporter,
            self.cancel,
        )
        .await?;
        if !outcome.success {
            let err = ffmpeg_failure("Decrypting", &outcome, true);
            return Err(self.on_source_failure(source, err).await);
        }
        let decoded = match self.validate(&target).await {
            Ok(info) => info,
            Err(err) => return Err(self.on_source_failure(source, err).await),
        };
        self.output_from_m4b(&target, &decoded, &plan_chapters).await
    }

    /// Where a decrypt/join writes: straight into the staged final file for M4B output,
    /// else an intermediate in the work dir (split into MP3s afterwards).
    fn intermediate_or_final(&mut self, container: Container) -> PathBuf {
        let ext = container.extension();
        match self.job.format {
            OutputFormat::M4b => {
                let final_path = self.job.dest_dir.join(format!("{}.{ext}", self.file_stem));
                self.audio.stage(final_path)
            }
            OutputFormat::Mp3 => {
                let path = self.work(&format!("book.{ext}"));
                self.scratch.push(path.clone());
                path
            }
        }
    }

    /// After a decrypt/join wrote and validated `path`: done for M4B output (it is
    /// already staged); for MP3 output split it by chapters, or — with no chapters —
    /// keep it as the single file.
    async fn output_from_m4b(
        &mut self,
        path: &Path,
        info: &ProbeInfo,
        plan_chapters: &[Chapter],
    ) -> Result<Produced, PipelineError> {
        match self.job.format {
            OutputFormat::M4b => Ok(Produced {
                chapter_count: info.chapters.len(),
                duration_ms: info.duration_ms(),
            }),
            OutputFormat::Mp3 => {
                let list = if plan_chapters.is_empty() { info.chapters.clone() } else { plan_chapters.to_vec() };
                if list.is_empty() {
                    log::info!("pipeline: no chapters; keeping a single file");
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("m4b");
                    let staged = self.audio.stage(self.job.dest_dir.join(format!("{}.{ext}", self.file_stem)));
                    link_or_copy(path, &staged).await?;
                    Ok(Produced { chapter_count: 0, duration_ms: info.duration_ms() })
                } else {
                    self.split_to_mp3(path, info, &list).await
                }
            }
        }
    }

    // ---------------------------------------------------------------- plain

    async fn plain_single(&mut self, source: &Source) -> Result<Produced, PipelineError> {
        let info = match self.validate(&source.path).await {
            Ok(info) => info,
            Err(err) => return Err(self.on_source_failure(source, err).await),
        };
        let plan_chapters = chapters::from_plan(&self.plan().chapters);

        if info.is_mp4() {
            let list = if plan_chapters.is_empty() { info.chapters.clone() } else { plan_chapters.clone() };
            if self.job.format == OutputFormat::Mp3 && !list.is_empty() {
                return self.split_to_mp3(&source.path, &info, &list).await;
            }
            let inject = if info.chapters.is_empty() && !plan_chapters.is_empty() {
                Some(self.write_chapter_file(&plan_chapters).await?)
            } else {
                None
            };
            let staged = self.audio.stage(self.job.dest_dir.join(format!("{}.m4b", self.file_stem)));
            if self.apply_tags || inject.is_some() {
                let tags = if self.apply_tags { args::m4b_tags(&self.job.meta) } else { Vec::new() };
                let spec = OutputSpec {
                    metadata: MetaSource::KeepSource,
                    chapters: inject.as_deref().map_or(ChapterSource::KeepSource, ChapterSource::File),
                    cover: if self.apply_tags { self.cover_arg() } else { None },
                    keep_source_cover: true,
                    tags,
                    audio: AudioCodec::Copy,
                    container: Container::M4b,
                    segment: None,
                };
                self.remux(&source.path, &spec, &staged, &info, "Tagging").await?;
            } else {
                self.copy_into_place(&source.path, &staged).await?;
            }
            let chapter_count = if inject.is_some() { plan_chapters.len() } else { info.chapters.len() };
            return Ok(Produced { chapter_count, duration_ms: info.duration_ms() });
        }

        if info.is_mp3() {
            let staged = self.audio.stage(self.job.dest_dir.join(format!("{}.mp3", self.file_stem)));
            if self.apply_tags {
                let spec = OutputSpec {
                    metadata: MetaSource::KeepSource,
                    chapters: ChapterSource::KeepSource,
                    cover: self.cover_arg(),
                    keep_source_cover: true,
                    tags: args::mp3_book_tags(&self.job.meta),
                    audio: AudioCodec::Copy,
                    container: Container::Mp3,
                    segment: None,
                };
                self.remux(&source.path, &spec, &staged, &info, "Tagging").await?;
            } else {
                self.copy_into_place(&source.path, &staged).await?;
            }
            return Ok(Produced { chapter_count: info.chapters.len(), duration_ms: info.duration_ms() });
        }

        // Anything else (Ogg, FLAC, …) is delivered untouched.
        let ext = source
            .path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_else(|| "audio".to_string());
        let staged = self.audio.stage(self.job.dest_dir.join(format!("{}.{ext}", self.file_stem)));
        self.copy_into_place(&source.path, &staged).await?;
        Ok(Produced { chapter_count: info.chapters.len(), duration_ms: info.duration_ms() })
    }

    async fn copy_into_place(&self, src: &Path, staged: &Path) -> Result<(), PipelineError> {
        self.reporter.stage(Stage::Finalizing, None);
        link_or_copy(src, staged).await
    }

    /// Stream-copy `src` to `dst` with new tags/cover/chapters.
    async fn remux(
        &self,
        src: &Path,
        spec: &OutputSpec<'_>,
        dst: &Path,
        info: &ProbeInfo,
        context: &str,
    ) -> Result<(), PipelineError> {
        require_free_space(parent_of(dst), file_size(src).await)?;
        self.reporter.stage(Stage::Finalizing, Some(0.0));
        let args = args::ffmpeg_args(&Input::Single { pre: Vec::new(), path: src }, spec, dst);
        let outcome = run_ffmpeg(
            self.tools,
            FfmpegJob {
                args,
                stage: Stage::Finalizing,
                duration_secs: info.duration_secs,
                span: (0.0, 1.0),
                count_decode_errors: false,
            },
            &self.reporter,
            self.cancel,
        )
        .await?;
        if outcome.success {
            Ok(())
        } else {
            Err(ffmpeg_failure(context, &outcome, false))
        }
    }

    // ---------------------------------------------------------------- join

    async fn join(&mut self, sources: &[Source]) -> Result<Produced, PipelineError> {
        let mut infos = Vec::with_capacity(sources.len());
        for source in sources {
            match probe(self.tools, &source.path, &[], self.cancel).await {
                Ok(info) if info.audio.is_some() => infos.push(info),
                Ok(_) => {
                    let err = PipelineError::new(
                        FailureKind::Validation,
                        format!("{} has no audio stream", source.path.display()),
                    );
                    return Err(self.on_source_failure(source, err).await);
                }
                Err(e) => {
                    let err = e.into_error(FailureKind::Validation, &source.path);
                    return Err(self.on_source_failure(source, err).await);
                }
            }
        }

        let first = infos[0].audio.clone().expect("checked above");
        let uniform = infos.iter().all(|i| {
            let a = i.audio.as_ref().expect("checked above");
            a.codec == first.codec && a.sample_rate == first.sample_rate && a.channels == first.channels
        });
        let (audio, container) = match (uniform, first.codec.as_str()) {
            (true, "aac") => (AudioCodec::Copy, Container::M4b),
            (true, "mp3") => (AudioCodec::Copy, Container::Mp3),
            _ => {
                let max_bps = infos.iter().filter_map(ProbeInfo::audio_bit_rate).max();
                log::info!(
                    "pipeline: parts differ in codec or format (or aren't AAC/MP3); re-encoding the join to AAC"
                );
                (AudioCodec::Aac { kbps: args::mp3_bitrate_kbps(max_bps) }, Container::M4b)
            }
        };

        // Chapters: the plan's, else one per part from the probed durations.
        let mut chapter_list = chapters::from_plan(&self.plan().chapters);
        if chapter_list.is_empty() {
            let mut cursor = 0u64;
            for (i, info) in infos.iter().enumerate() {
                let length = info.duration_ms().unwrap_or(0);
                chapter_list.push(Chapter {
                    title: format!("Part {}", i + 1),
                    start_ms: cursor,
                    end_ms: cursor + length,
                });
                cursor += length;
            }
            chapter_list = chapters::normalise(chapter_list);
        }
        let chapter_file = if chapter_list.is_empty() {
            None
        } else {
            Some(self.write_chapter_file(&chapter_list).await?)
        };

        let paths: Vec<&Path> = sources.iter().map(|s| s.path.as_path()).collect();
        let list_path;
        let input = if audio == AudioCodec::Copy {
            list_path = self.work("join.txt");
            self.scratch.push(list_path.clone());
            tokio::fs::write(&list_path, chapters::concat_list(&paths))
                .await
                .map_err(|e| io_error("writing the join list", e))?;
            Input::Concat { list: &list_path }
        } else {
            Input::ConcatFilter { paths: paths.clone() }
        };

        let target = self.intermediate_or_final(container);
        let mut total_size = 0u64;
        for path in &paths {
            total_size += file_size(path).await;
        }
        require_free_space(parent_of(&target), total_size)?;
        self.reporter.stage(Stage::Joining, Some(0.0));
        log::info!("pipeline: joining {} parts", paths.len());

        let tags = match container {
            Container::M4b => args::m4b_tags(&self.job.meta),
            Container::Mp3 => args::mp3_book_tags(&self.job.meta),
        };
        let spec = OutputSpec {
            metadata: MetaSource::Reset,
            chapters: chapter_file.as_deref().map_or(ChapterSource::Drop, ChapterSource::File),
            cover: self.cover_arg(),
            keep_source_cover: false,
            tags,
            audio,
            container,
            segment: None,
        };
        let total_secs: f64 = infos.iter().filter_map(|i| i.duration_secs).sum();
        let args = args::ffmpeg_args(&input, &spec, &target);
        let outcome = run_ffmpeg(
            self.tools,
            FfmpegJob {
                args,
                stage: Stage::Joining,
                duration_secs: (total_secs > 0.0).then_some(total_secs),
                span: (0.0, 1.0),
                count_decode_errors: false,
            },
            &self.reporter,
            self.cancel,
        )
        .await?;
        if !outcome.success {
            return Err(ffmpeg_failure("Joining tracks", &outcome, false));
        }
        let joined = self.validate(&target).await?;
        let plan_chapters = chapters::from_plan(&self.plan().chapters);
        self.output_from_m4b(&target, &joined, if plan_chapters.is_empty() { &chapter_list } else { &plan_chapters })
            .await
    }

    async fn write_chapter_file(&mut self, list: &[Chapter]) -> Result<PathBuf, PipelineError> {
        let path = self.work("chapters.ffmeta");
        tokio::fs::write(&path, chapters::ffmetadata(list))
            .await
            .map_err(|e| io_error("writing the chapter file", e))?;
        self.scratch.push(path.clone());
        Ok(path)
    }

    // ---------------------------------------------------------------- MP3 per chapter

    /// Port of `encodeChaptersToMp3`: one `NN - Title.mp3` per chapter, ID3v2.3 tags,
    /// cover as APIC. An MP3 source is cut with a stream copy (no lossy re-encode).
    async fn split_to_mp3(
        &mut self,
        source: &Path,
        info: &ProbeInfo,
        list: &[Chapter],
    ) -> Result<Produced, PipelineError> {
        let total = list.len();
        let audio = if info.audio.as_ref().map(|a| a.codec.as_str()) == Some("mp3") {
            AudioCodec::Copy
        } else {
            AudioCodec::Mp3 { kbps: args::mp3_bitrate_kbps(info.audio_bit_rate()) }
        };
        require_free_space(&self.job.dest_dir, file_size(source).await)?;
        self.reporter.stage(Stage::Encoding, Some(0.0));
        log::info!("pipeline: writing {total} chapter MP3s ({audio:?})");

        let total_ms: u64 = list.iter().map(Chapter::duration_ms).sum::<u64>().max(1);
        let mut done_ms = 0u64;
        for (i, chapter) in list.iter().enumerate() {
            self.check_cancel()?;
            let n = i + 1;
            let name = chapters::chapter_file_name(n, total, &chapter.title);
            let staged = self.audio.stage(self.job.dest_dir.join(&name));
            let spec = OutputSpec {
                metadata: MetaSource::Reset,
                chapters: ChapterSource::Drop,
                cover: self.cover_arg(),
                keep_source_cover: false,
                tags: args::mp3_chapter_tags(&self.job.meta, &chapter.title, n, total),
                audio,
                container: Container::Mp3,
                segment: Some((
                    chapter.start_ms as f64 / 1000.0,
                    chapter.duration_ms() as f64 / 1000.0,
                )),
            };
            let args = args::ffmpeg_args(&Input::Single { pre: Vec::new(), path: source }, &spec, &staged);
            let outcome = run_ffmpeg(
                self.tools,
                FfmpegJob {
                    args,
                    stage: Stage::Encoding,
                    duration_secs: Some(chapter.duration_ms() as f64 / 1000.0),
                    span: (
                        done_ms as f32 / total_ms as f32,
                        chapter.duration_ms() as f32 / total_ms as f32,
                    ),
                    count_decode_errors: false,
                },
                &self.reporter,
                self.cancel,
            )
            .await?;
            if !outcome.success {
                return Err(ffmpeg_failure(&format!("Encoding chapter {n} of {total}"), &outcome, false));
            }
            done_ms += chapter.duration_ms();
        }
        Ok(Produced { chapter_count: total, duration_ms: info.duration_ms() })
    }

    // ---------------------------------------------------------------- ZIP / multi-file

    /// ZIP parts are extracted and plain parts copied, as the store delivered them —
    /// these are already per-track files, so neither output format transcodes them.
    async fn collection(&mut self, sources: &[Source]) -> Result<Produced, PipelineError> {
        self.reporter.stage(Stage::Extracting, Some(0.0));
        let mut taken: HashSet<String> = HashSet::new();
        let mut files: Vec<PathBuf> = Vec::new();
        let job = self.job;
        for (part, source) in job.plan.parts.iter().zip(sources) {
            self.check_cancel()?;
            match part {
                DownloadPart::Zip { .. } => {
                    let zip_path = source.path.clone();
                    let dest = self.job.dest_dir.clone();
                    let reporter = Arc::clone(&self.reporter);
                    let cancel = self.cancel.clone();
                    let mut names = std::mem::take(&mut taken);
                    let result = tokio::task::spawn_blocking(move || {
                        let r = extract::extract_audio(&zip_path, &dest, &mut names, &reporter, &cancel);
                        (r, names)
                    })
                    .await
                    .map_err(|e| PipelineError::new(FailureKind::Io, format!("extraction task failed: {e}")))?;
                    taken = result.1;
                    let pairs = match result.0 {
                        Ok(pairs) => pairs,
                        Err(err) => return Err(self.on_source_failure(source, err).await),
                    };
                    for (temp, final_path) in pairs {
                        self.audio.adopt(temp.clone(), final_path);
                        files.push(temp);
                    }
                }
                DownloadPart::Plain { .. } => {
                    let leaf = source.path.file_name().and_then(|n| n.to_str()).unwrap_or("track");
                    let name = extract::unique_name(leaf, &mut taken);
                    let staged = self.audio.stage(self.job.dest_dir.join(name));
                    link_or_copy(&source.path, &staged).await?;
                    files.push(staged);
                }
                DownloadPart::Aaxc { .. } | DownloadPart::Aax { .. } => {
                    unreachable!("check_plan rejects encrypted parts in multi-part plans")
                }
            }
        }
        let mut duration_ms = 0u64;
        let mut known = true;
        for file in &files {
            match probe(self.tools, file, &[], self.cancel).await {
                Ok(info) => match info.duration_ms() {
                    Some(ms) => duration_ms += ms,
                    None => known = false,
                },
                Err(tools::ProbeError::Tool(e)) if e.kind == FailureKind::Cancelled => return Err(e),
                Err(_) => known = false,
            }
        }
        Ok(Produced { chapter_count: files.len(), duration_ms: known.then_some(duration_ms) })
    }

    // ---------------------------------------------------------------- finish

    async fn finish(&mut self, produced: Produced) -> Result<PipelineOutput, PipelineError> {
        self.reporter.stage(Stage::Finalizing, None);
        if self.job.write_sidecars {
            if let Some(cover) = self.cover.clone() {
                let staged = self
                    .sidecars
                    .stage(self.job.dest_dir.join(format!("cover.{}", cover.extension)));
                link_or_copy(&cover.path, &staged).await?;
            }
            for (name, contents) in sidecars::text_sidecars(&self.job.meta) {
                let staged = self.sidecars.stage(self.job.dest_dir.join(name));
                sidecars::write_file(&staged, contents.as_bytes()).await?;
            }
        }
        self.check_cancel()?;

        std::mem::take(&mut self.sidecars).commit().await?;
        let audio_files = std::mem::take(&mut self.audio).commit().await?;
        remove_stale_outputs(&self.job.dest_dir, &self.file_stem, &audio_files).await?;

        let mut total_bytes = 0u64;
        for file in &audio_files {
            total_bytes += file_size(file).await;
        }
        Ok(PipelineOutput {
            dest_dir: self.job.dest_dir.clone(),
            audio_files,
            total_bytes,
            duration_ms: produced.duration_ms,
            chapter_count: produced.chapter_count,
            format: self.job.format,
        })
    }
}

fn parent_of(path: &Path) -> &Path {
    path.parent().unwrap_or(Path::new("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::PlanChapter;

    fn plain(filename: &str) -> DownloadPart {
        DownloadPart::Plain { url: "http://x/y".into(), headers: Default::default(), filename: filename.into() }
    }

    fn plan(parts: Vec<DownloadPart>, join: bool) -> DownloadPlan {
        DownloadPlan { parts, embed_metadata: true, chapters: Vec::<PlanChapter>::new(), join_parts: join }
    }

    #[test]
    fn plans_are_checked() {
        assert_eq!(check_plan(&plan(vec![], false)).unwrap_err().kind, FailureKind::InvalidPlan);
        let zip = DownloadPart::Zip { url: "http://x/z".into(), headers: Default::default() };
        assert_eq!(
            check_plan(&plan(vec![plain("1.m4a"), zip.clone()], true)).unwrap_err().kind,
            FailureKind::InvalidPlan
        );
        let aax = DownloadPart::Aax {
            url: "http://x/a".into(),
            headers: Default::default(),
            activation_bytes: "00".into(),
            filename: "a.aax".into(),
        };
        assert_eq!(
            check_plan(&plan(vec![aax.clone(), zip.clone()], false)).unwrap_err().kind,
            FailureKind::InvalidPlan
        );
        assert!(check_plan(&plan(vec![aax], false)).is_ok());
        assert!(check_plan(&plan(vec![zip.clone(), zip], false)).is_ok());
        assert!(check_plan(&plan(vec![plain("1.m4a"), plain("2.m4a")], true)).is_ok());
    }

    #[test]
    fn work_names_are_safe_and_unique() {
        let zip = DownloadPart::Zip { url: "http://x/z".into(), headers: Default::default() };
        assert_eq!(
            work_file_names(&plan(vec![plain("001.m4a"), plain("002.mp3")], true)),
            vec!["001.m4a", "002.mp3"]
        );
        assert_eq!(
            work_file_names(&plan(vec![plain("../../etc/passwd"), zip.clone(), zip], false)),
            vec!["passwd", "part002.zip", "part003.zip"]
        );
        assert_eq!(
            work_file_names(&plan(vec![plain("a.mp3"), plain("a.mp3")], false)),
            vec!["001-a.mp3", "002-a.mp3"]
        );
        assert_eq!(
            work_file_names(&plan(vec![plain(".hidden.m4a")], false)),
            vec!["part001-hidden.m4a"]
        );
    }
}
