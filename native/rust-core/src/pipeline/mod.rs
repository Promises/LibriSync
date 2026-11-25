//! Download pipeline for hosts that run the system `ffmpeg` (the LibriSync server; later
//! the iOS port and, potentially, the Android app).
//!
//! Executes one provider [`DownloadPlan`] end to end: resumable download of every part,
//! AAXC/AAX decryption, ZIP extraction, joining per-track parts, full-decode validation,
//! output as one chaptered M4B or one MP3 per chapter, tags and cover, and Audiobookshelf
//! sidecar files. It is deliberately stateless — no database access. The caller owns
//! queueing, retries, licence pacing and recording the result.
//!
//! Port of the Android engine in
//! `modules/expo-rust-bridge/android/.../DownloadOrchestrator.kt` (`enqueuePlan`,
//! `triggerConversion`, `triggerPlainAudioCopy`, `joinParts`, `encodeChaptersToMp3`).
//!
//! Layout:
//! - `exec` — stage sequencing per plan shape (the orchestrator port)
//! - `download` — resumable, verified HTTP download of one part
//! - `tools` — `ffmpeg`/`ffprobe` capability check, probing, cancellable runner
//! - `args` — tag sets and ffmpeg argument vectors
//! - `chapters` — FFMETADATA chapter file, concat list, `NN - Title.mp3` names
//! - `extract` — ZIP extraction
//! - `sidecars` — cover download, `desc.txt`, `reader.txt`, `metadata.opf`
//! - `fsutil` — free space, error classification, sanitising, atomic placement
//! - `progress` — throttled progress reporting
//!
//! Logging goes through the `log` facade (`pipeline:` prefix); the host installs the
//! logger.

mod args;
mod chapters;
mod download;
mod exec;
mod extract;
mod fsutil;
mod progress;
mod sidecars;
mod tools;

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::providers::DownloadPlan;

/// Prefix of the pipeline's own files in the work dir (kept apart from part files).
pub(crate) const WORK_PREFIX: &str = "librisync.";

/// Book metadata used for tags, sidecars and file names. Built by the caller from the
/// core DB; every field except `product_id` and `title` is optional.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BookMeta {
    /// `Books.audible_product_id` (ASIN, `chirp_<id>`, ISBN).
    pub product_id: String,
    /// Book title (`title` and `album` tags, OPF `dc:title`).
    pub title: String,
    /// Appended to the M4B `comment` tag as Android did.
    pub subtitle: Option<String>,
    /// `artist`/`album_artist` tags (joined with ", "), OPF creators with role `aut`.
    pub authors: Vec<String>,
    /// `composer` tag, `reader.txt`, OPF creators with role `nrt`.
    pub narrators: Vec<String>,
    /// Series name (`show` in M4B, `series` in MP3, `calibre:series` in OPF).
    pub series: Option<String>,
    /// As the store gives it ("1", "2.5"), never reformatted as a float.
    pub series_sequence: Option<String>,
    /// Publisher (MP3 `publisher` tag, OPF, and the M4B `copyright` line with a year).
    pub publisher: Option<String>,
    /// `YYYY-MM-DD` or `YYYY`.
    pub release_date: Option<String>,
    /// Description (`description`/`comment` tags, `desc.txt`, OPF).
    pub description: Option<String>,
    /// Language name as the store gives it (OPF `dc:language`).
    pub language: Option<String>,
    /// Genres (`genre` tag joined with "; ", OPF `dc:subject`). Empty → "Audiobook".
    pub genres: Vec<String>,
    /// Audible ASIN (MP3 `asin` tag, OPF identifier).
    pub asin: Option<String>,
    /// ISBN (MP3 `isbn` tag, OPF identifier).
    pub isbn: Option<String>,
    /// Cover image URL. Downloaded with the caller's client, embedded and written as
    /// `cover.<jpg|png|webp>`; a failed download is logged and skipped.
    pub cover_url: Option<String>,
}

/// The user's output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    /// One chaptered `.m4b` (Audible/Chirp), or the store's own single file.
    M4b,
    /// One `NN - Chapter Title.mp3` per chapter.
    Mp3,
}

/// Everything [`run`] needs for one book.
#[derive(Debug, Clone)]
pub struct PipelineJob {
    pub plan: DownloadPlan,
    pub meta: BookMeta,
    pub format: OutputFormat,
    /// Scratch directory for this book. Partial downloads left here by an earlier,
    /// interrupted run are resumed when the part file names match. Removed on success.
    pub work_dir: PathBuf,
    /// The finished book's folder (absolute). Created if missing. Audio files are written
    /// to a temp name inside it and renamed into place, so a crash never leaves a
    /// truncated file under the final name.
    pub dest_dir: PathBuf,
    /// File name (without extension) for single-file output, e.g. the sanitised title.
    pub file_stem: String,
    /// Write `cover.jpg`, `desc.txt`, `reader.txt` and `metadata.opf` into `dest_dir`.
    pub write_sidecars: bool,
}

/// Pipeline stages, reported through [`Progress`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Downloading,
    Decrypting,
    Extracting,
    Joining,
    Validating,
    Encoding,
    Finalizing,
}

/// A progress report. Download progress is reported at most about once per second.
#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub stage: Stage,
    /// 1-based index of the part being downloaded (1 outside `Downloading`).
    pub part: usize,
    pub parts: usize,
    pub bytes_done: u64,
    pub bytes_total: Option<u64>,
    pub bytes_per_sec: Option<u64>,
    /// 0..=1 within the current stage, when known.
    pub fraction: Option<f32>,
}

/// What a successful run produced.
#[derive(Debug, Clone, Serialize)]
pub struct PipelineOutput {
    pub dest_dir: PathBuf,
    /// Audio files written, in playback order.
    pub audio_files: Vec<PathBuf>,
    /// Sum of the audio files' sizes.
    pub total_bytes: u64,
    pub duration_ms: Option<u64>,
    pub chapter_count: usize,
    pub format: OutputFormat,
}

/// Why a run failed. The caller decides retries from this, not from the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// Connection reset, timeout, DNS — retry later.
    Network,
    /// Unexpected HTTP status (other than the `Expired` ones).
    Http,
    /// A signed URL was refused (401/403/410) — the plan must be fetched again.
    Expired,
    /// Not enough free space (checked up front, and ENOSPC during writes).
    DiskFull,
    /// ffmpeg/ffprobe failed or is missing.
    Ffmpeg,
    /// The decoded audio failed validation (truncated or corrupt source).
    Validation,
    /// The plan itself is unusable (e.g. `join_parts` with a non-plain part).
    InvalidPlan,
    /// Other local I/O failure.
    Io,
    /// The cancellation token fired.
    Cancelled,
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct PipelineError {
    pub kind: FailureKind,
    pub message: String,
}

impl PipelineError {
    pub fn new(kind: FailureKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }

    /// Worth retrying the same book later without user action.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self.kind,
            FailureKind::Network | FailureKind::Http | FailureKind::Expired | FailureKind::Validation
        )
    }
}

/// Locations of the `ffmpeg`/`ffprobe` binaries.
#[derive(Debug, Clone)]
pub struct MediaTools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

impl MediaTools {
    /// `FFMPEG_PATH` / `FFPROBE_PATH`, else `ffmpeg` / `ffprobe` on `PATH`.
    pub fn from_env() -> Self {
        Self::resolve_from_env()
    }

    /// Run `ffmpeg -version`; returns the version string (e.g. `7.1.1-1`, the token
    /// after "ffmpeg version"). Fails with [`FailureKind::Ffmpeg`] if either binary is
    /// missing or ffmpeg lacks what the pipeline needs: the `libmp3lame` and `aac`
    /// encoders, the mov demuxer's `audible_key`/`activation_bytes` options (FFmpeg ≥ 4.4)
    /// and the concat demuxer.
    pub async fn check(&self) -> Result<String, PipelineError> {
        self.run_check().await
    }
}

/// Progress callback. Called from the pipeline's task; must not block.
pub type ProgressFn = Arc<dyn Fn(Progress) + Send + Sync>;

/// Execute `job`. On success `work_dir` is removed; on failure or cancellation the
/// downloaded parts stay in it for the next attempt (temp outputs in `dest_dir` are
/// removed). `http` is used for every download (parts and cover) so the caller
/// controls timeouts and proxies; the plan's per-part headers are added per request.
pub async fn run(
    job: PipelineJob,
    tools: &MediaTools,
    http: &reqwest::Client,
    on_progress: ProgressFn,
    cancel: CancellationToken,
) -> Result<PipelineOutput, PipelineError> {
    exec::execute(job, tools, http, on_progress, cancel).await
}
