//! Running the system `ffmpeg`/`ffprobe`: capability check, probing, and a cancellable
//! runner that reports progress from `-progress pipe:1`.
//!
//! Replaces the FFmpeg-Kit calls in `DownloadOrchestrator.kt` / `AudioValidation.kt`
//! (`FFmpegKit.executeAsync`, `FFprobeKit.getMediaInformation`, the statistics and log
//! callbacks) with plain processes: an argv (no string quoting), `-nostdin` and a null
//! stdin, and SIGKILL on cancel.

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use super::chapters::{self, Chapter};
use super::progress::Reporter;
use super::{FailureKind, MediaTools, PipelineError, Stage};

/// Stderr lines kept for error messages.
const STDERR_TAIL_LINES: usize = 20;
/// Validation gives up after this many decode errors (Android's limit).
const MAX_DECODE_ERRORS: usize = 50;
/// Timeout for the quick capability checks in [`MediaTools::check`].
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) fn cancelled() -> PipelineError {
    PipelineError::new(FailureKind::Cancelled, "Cancelled")
}

fn spawn_error(program: &Path, e: std::io::Error) -> PipelineError {
    let hint = if e.kind() == std::io::ErrorKind::NotFound {
        " (install ffmpeg or set FFMPEG_PATH / FFPROBE_PATH)"
    } else {
        ""
    };
    PipelineError::new(
        FailureKind::Ffmpeg,
        format!("Could not run {}: {e}{hint}", program.display()),
    )
}

impl MediaTools {
    pub(crate) fn resolve_from_env() -> Self {
        let pick = |var: &str, default: &str| {
            std::env::var_os(var)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(default))
        };
        Self { ffmpeg: pick("FFMPEG_PATH", "ffmpeg"), ffprobe: pick("FFPROBE_PATH", "ffprobe") }
    }

    pub(crate) async fn run_check(&self) -> Result<String, PipelineError> {
        let version_text = capture(&self.ffmpeg, &["-hide_banner", "-version"]).await?;
        let first_line = version_text.lines().next().unwrap_or_default().trim().to_string();
        capture(&self.ffprobe, &["-hide_banner", "-version"]).await?;

        let encoders = capture(&self.ffmpeg, &["-hide_banner", "-encoders"]).await?;
        let has_encoder = |name: &str| {
            encoders
                .lines()
                .any(|line| line.split_whitespace().nth(1) == Some(name))
        };
        if !has_encoder("libmp3lame") {
            return Err(PipelineError::new(
                FailureKind::Ffmpeg,
                format!(
                    "{first_line} has no libmp3lame encoder, which MP3 output needs. \
                     Use an ffmpeg built with --enable-libmp3lame (Debian's ffmpeg package has it)."
                ),
            ));
        }
        if !has_encoder("aac") {
            return Err(PipelineError::new(
                FailureKind::Ffmpeg,
                format!("{first_line} has no aac encoder."),
            ));
        }

        let mov = capture(&self.ffmpeg, &["-hide_banner", "-h", "demuxer=mov"]).await?;
        if !mov.contains("audible_key") || !mov.contains("activation_bytes") {
            return Err(PipelineError::new(
                FailureKind::Ffmpeg,
                format!(
                    "{first_line} cannot decrypt Audible files (no -audible_key option). \
                     FFmpeg 4.4 or newer is required."
                ),
            ));
        }
        let concat = capture(&self.ffmpeg, &["-hide_banner", "-h", "demuxer=concat"]).await?;
        if !concat.contains("Demuxer concat") {
            return Err(PipelineError::new(
                FailureKind::Ffmpeg,
                format!("{first_line} lacks the concat demuxer."),
            ));
        }

        // "ffmpeg version 7.1.1-1 Copyright (c) ..." → "7.1.1-1".
        let version = first_line
            .strip_prefix("ffmpeg version ")
            .and_then(|rest| rest.split_whitespace().next())
            .map(str::to_string)
            .unwrap_or(first_line);
        Ok(version)
    }
}

/// Run a short tool invocation and return its stdout.
async fn capture(program: &Path, args: &[&str]) -> Result<String, PipelineError> {
    let output = tokio::time::timeout(
        CHECK_TIMEOUT,
        Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| {
        PipelineError::new(
            FailureKind::Ffmpeg,
            format!("{} did not answer within {CHECK_TIMEOUT:?}", program.display()),
        )
    })?
    .map_err(|e| spawn_error(program, e))?;
    if !output.status.success() {
        return Err(PipelineError::new(
            FailureKind::Ffmpeg,
            format!(
                "{} {} failed: {}",
                program.display(),
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The audio stream ffprobe reports.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AudioStreamInfo {
    pub codec: String,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    pub bit_rate: Option<u64>,
}

/// What ffprobe says about a file.
#[derive(Debug, Clone)]
pub(crate) struct ProbeInfo {
    pub format_name: String,
    pub duration_secs: Option<f64>,
    pub bit_rate: Option<u64>,
    pub audio: Option<AudioStreamInfo>,
    pub chapters: Vec<Chapter>,
}

impl ProbeInfo {
    /// The audio bitrate (stream, else container).
    pub(crate) fn audio_bit_rate(&self) -> Option<u64> {
        self.audio.as_ref().and_then(|a| a.bit_rate).or(self.bit_rate)
    }

    /// MP4 family (`mov,mp4,m4a,3gp,3g2,mj2`).
    pub(crate) fn is_mp4(&self) -> bool {
        self.format_name.split(',').any(|f| f == "mp4" || f == "mov")
    }

    /// MP3 container with MP3 audio.
    pub(crate) fn is_mp3(&self) -> bool {
        self.format_name == "mp3"
    }

    pub(crate) fn duration_ms(&self) -> Option<u64> {
        self.duration_secs.map(|s| (s * 1000.0).round() as u64)
    }
}

/// Why a probe failed.
#[derive(Debug)]
pub(crate) enum ProbeError {
    /// ffprobe couldn't run (missing binary, cancelled).
    Tool(PipelineError),
    /// ffprobe ran and couldn't read the file.
    Unreadable(String),
}

impl ProbeError {
    /// Treat an unreadable file as `kind` (Validation for a downloaded source).
    pub(crate) fn into_error(self, kind: FailureKind, what: &Path) -> PipelineError {
        match self {
            ProbeError::Tool(e) => e,
            ProbeError::Unreadable(msg) => PipelineError::new(
                kind,
                format!("Could not read {}: {msg}", what.display()),
            ),
        }
    }
}

fn parse_num<T: std::str::FromStr>(value: Option<&serde_json::Value>) -> Option<T> {
    match value? {
        serde_json::Value::String(s) => s.parse().ok(),
        serde_json::Value::Number(n) => n.to_string().parse().ok(),
        _ => None,
    }
}

/// Parse `ffprobe -print_format json -show_format -show_streams -show_chapters`.
pub(crate) fn parse_probe_json(text: &str) -> Result<ProbeInfo, String> {
    let json: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("unparseable ffprobe output: {e}"))?;
    let format = json.get("format").ok_or("ffprobe reported no format")?;
    let audio = json
        .get("streams")
        .and_then(|s| s.as_array())
        .and_then(|streams| {
            streams
                .iter()
                .find(|s| s.get("codec_type").and_then(|t| t.as_str()) == Some("audio"))
        })
        .map(|s| AudioStreamInfo {
            codec: s.get("codec_name").and_then(|c| c.as_str()).unwrap_or_default().to_string(),
            sample_rate: parse_num(s.get("sample_rate")),
            channels: parse_num(s.get("channels")),
            bit_rate: parse_num(s.get("bit_rate")),
        });
    let probed_chapters = json
        .get("chapters")
        .and_then(|c| c.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|c| {
                    let start: f64 = parse_num(c.get("start_time"))?;
                    let end: f64 = parse_num(c.get("end_time"))?;
                    let title = c
                        .get("tags")
                        .and_then(|t| t.get("title"))
                        .and_then(|t| t.as_str())
                        .unwrap_or_default()
                        .to_string();
                    Some(Chapter {
                        title,
                        start_ms: (start.max(0.0) * 1000.0).round() as u64,
                        end_ms: (end.max(0.0) * 1000.0).round() as u64,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(ProbeInfo {
        format_name: format
            .get("format_name")
            .and_then(|f| f.as_str())
            .unwrap_or_default()
            .to_string(),
        duration_secs: parse_num::<f64>(format.get("duration")).filter(|d| d.is_finite()),
        bit_rate: parse_num(format.get("bit_rate")),
        audio,
        chapters: chapters::normalise(probed_chapters),
    })
}

/// ffprobe a file. `pre` holds demuxer options (DRM keys) for an encrypted input.
pub(crate) async fn probe(
    tools: &MediaTools,
    path: &Path,
    pre: &[OsString],
    cancel: &CancellationToken,
) -> Result<ProbeInfo, ProbeError> {
    let mut cmd = Command::new(&tools.ffprobe);
    cmd.args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams", "-show_chapters"])
        .args(pre)
        .arg("-i")
        .arg(path)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::select! {
        _ = cancel.cancelled() => return Err(ProbeError::Tool(cancelled())),
        out = cmd.output() => out.map_err(|e| ProbeError::Tool(spawn_error(&tools.ffprobe, e)))?,
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ProbeError::Unreadable(last_lines(&stderr, 3)));
    }
    parse_probe_json(&String::from_utf8_lossy(&output.stdout)).map_err(ProbeError::Unreadable)
}

fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join(" | ")
}

/// One ffmpeg invocation.
pub(crate) struct FfmpegJob {
    /// Arguments after the runner's common flags (see [`run_ffmpeg`]).
    pub args: Vec<OsString>,
    pub stage: Stage,
    /// Seconds of audio this run processes, for the stage fraction.
    pub duration_secs: Option<f64>,
    /// This run's 0..1 is reported as `offset + fraction * scale` of the stage.
    pub span: (f32, f32),
    /// Count decode-error lines on stderr and abort after [`MAX_DECODE_ERRORS`]
    /// (validation).
    pub count_decode_errors: bool,
}

/// How an ffmpeg run ended (when not cancelled and the binary ran).
#[derive(Debug)]
pub(crate) struct FfmpegOutcome {
    pub success: bool,
    pub stderr_tail: String,
    pub decode_errors: usize,
    /// The last `out_time` reported (seconds of output written / decoded).
    pub out_time_secs: Option<f64>,
}

/// A stderr line that signals a decode problem. Android counted messages containing
/// "error" or "invalid data"; "partial file" and "corrupt" are what ffmpeg prints for a
/// truncated MP4 and are added here.
pub(crate) fn is_decode_error_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    ["error", "invalid data", "partial file", "corrupt", "truncat"]
        .iter()
        .any(|needle| lower.contains(needle))
}

/// Run ffmpeg with `-nostdin -hide_banner -nostats -loglevel error -progress pipe:1 -y`
/// before `job.args`. Cancellation kills the child and returns `Cancelled`.
pub(crate) async fn run_ffmpeg(
    tools: &MediaTools,
    job: FfmpegJob,
    reporter: &Reporter,
    cancel: &CancellationToken,
) -> Result<FfmpegOutcome, PipelineError> {
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    let mut cmd = Command::new(&tools.ffmpeg);
    cmd.args([
        "-nostdin", "-hide_banner", "-nostats", "-loglevel", "error", "-progress", "pipe:1", "-y",
    ])
    .args(&job.args)
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);
    log::debug!("pipeline: {} {}", tools.ffmpeg.display(), display_args(&job.args));
    let mut child = cmd.spawn().map_err(|e| spawn_error(&tools.ffmpeg, e))?;

    let mut stdout = child.stdout.take().map(|s| BufReader::new(s).lines());
    let mut stderr = child.stderr.take().map(|s| BufReader::new(s).lines());
    let mut tail: VecDeque<String> = VecDeque::with_capacity(STDERR_TAIL_LINES);
    let mut decode_errors = 0usize;
    let mut out_time_secs: Option<f64> = None;
    let mut aborted = false;
    let (offset, scale) = job.span;

    while stdout.is_some() || stderr.is_some() {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = child.kill().await;
                return Err(cancelled());
            }
            line = async { stdout.as_mut().unwrap().next_line().await }, if stdout.is_some() => {
                match line {
                    Ok(Some(line)) => {
                        if let Some(secs) = parse_out_time(&line) {
                            out_time_secs = Some(secs);
                            if let Some(total) = job.duration_secs.filter(|t| *t > 0.0) {
                                let f = (secs / total).clamp(0.0, 1.0) as f32;
                                reporter.fraction(job.stage, offset + f * scale);
                            }
                        }
                    }
                    _ => stdout = None,
                }
            }
            line = async { stderr.as_mut().unwrap().next_line().await }, if stderr.is_some() => {
                match line {
                    Ok(Some(line)) => {
                        if job.count_decode_errors && is_decode_error_line(&line) {
                            decode_errors += 1;
                        }
                        if tail.len() == STDERR_TAIL_LINES {
                            tail.pop_front();
                        }
                        tail.push_back(line);
                        if decode_errors > MAX_DECODE_ERRORS {
                            let _ = child.kill().await;
                            aborted = true;
                            break;
                        }
                    }
                    _ => stderr = None,
                }
            }
        }
    }

    let status = tokio::select! {
        _ = cancel.cancelled() => {
            let _ = child.kill().await;
            return Err(cancelled());
        }
        status = child.wait() => status.map_err(|e| PipelineError::new(FailureKind::Ffmpeg, format!("waiting for ffmpeg: {e}")))?,
    };
    Ok(FfmpegOutcome {
        success: status.success() && !aborted,
        stderr_tail: tail.into_iter().collect::<Vec<_>>().join("\n"),
        decode_errors,
        out_time_secs,
    })
}

/// `out_time_us=` (or the misnamed `out_time_ms=`, also microseconds) → seconds.
fn parse_out_time(line: &str) -> Option<f64> {
    let value = line
        .strip_prefix("out_time_us=")
        .or_else(|| line.strip_prefix("out_time_ms="))?;
    value.trim().parse::<i64>().ok().filter(|v| *v >= 0).map(|us| us as f64 / 1_000_000.0)
}

/// ffmpeg flags whose value is a decryption secret (see `args::drm_input_args`).
const SECRET_FLAGS: [&str; 3] = ["-audible_key", "-audible_iv", "-activation_bytes"];

/// An argv for logs and error messages: quoted where it has spaces, and with the value
/// after every [`SECRET_FLAGS`] flag replaced by `***`, so DRM keys never reach a log
/// file, a queue error or a webhook.
fn display_args(args: &[OsString]) -> String {
    let mut redact_next = false;
    args.iter()
        .map(|a| {
            let s = a.to_string_lossy();
            let shown = if redact_next {
                "***".to_string()
            } else if s.contains(' ') {
                format!("'{s}'")
            } else {
                s.to_string()
            };
            redact_next = SECRET_FLAGS.contains(&s.as_ref());
            shown
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Port of Android's `ffmpegFailureMessage`: turn a failed run's stderr into an error.
/// `context` names the step ("Decrypting", "Joining tracks", …).
pub(crate) fn ffmpeg_failure(context: &str, outcome: &FfmpegOutcome, decrypting: bool) -> PipelineError {
    let lower = outcome.stderr_tail.to_ascii_lowercase();
    let detail = if outcome.stderr_tail.is_empty() {
        "no error output".to_string()
    } else {
        last_lines(&outcome.stderr_tail, 4)
    };
    if lower.contains("no space left") {
        return PipelineError::new(
            FailureKind::DiskFull,
            format!("{context} failed: ran out of storage ({detail})"),
        );
    }
    if lower.contains("permission denied") {
        return PipelineError::new(
            FailureKind::Io,
            format!("{context} failed: permission denied ({detail})"),
        );
    }
    if decrypting && (lower.contains("activation") || lower.contains("invalid data found")) {
        // A wrong key and a damaged download look the same; both deserve a fresh plan
        // (new licence, new download), so this is retryable.
        return PipelineError::new(
            FailureKind::Validation,
            format!("Decryption failed — the key may be wrong or the download damaged ({detail})"),
        );
    }
    if lower.contains("invalid data found") {
        return PipelineError::new(
            FailureKind::Validation,
            format!("{context} failed: the source audio is damaged ({detail})"),
        );
    }
    PipelineError::new(FailureKind::Ffmpeg, format!("{context} failed: {detail}"))
}

/// Path helper for log messages.
pub(crate) fn name_of(path: &Path) -> &OsStr {
    path.file_name().unwrap_or(path.as_os_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logged_args_hide_drm_secrets() {
        use crate::pipeline::args::{drm_input_args, Drm};
        let mut args = drm_input_args(Drm::Aaxc {
            key: "00112233445566778899aabbccddeeff",
            iv: "ffeeddccbbaa99887766554433221100",
        });
        args.extend(drm_input_args(Drm::Aax { activation_bytes: "1a2b3c4d" }));
        args.extend(["-i".into(), "/tmp/my book.aaxc".into()]);
        let shown = display_args(&args);
        assert_eq!(
            shown,
            "-audible_key *** -audible_iv *** -activation_bytes *** -i '/tmp/my book.aaxc'"
        );
        for secret in [
            "00112233445566778899aabbccddeeff",
            "ffeeddccbbaa99887766554433221100",
            "1a2b3c4d",
        ] {
            assert!(!shown.contains(secret), "{secret} leaked: {shown}");
        }
    }

    #[test]
    fn out_time_parsing() {
        assert_eq!(parse_out_time("out_time_us=1500000"), Some(1.5));
        assert_eq!(parse_out_time("out_time_ms=2000000"), Some(2.0));
        assert_eq!(parse_out_time("out_time_us=N/A"), None);
        assert_eq!(parse_out_time("out_time=00:00:01.500000"), None);
        assert_eq!(parse_out_time("progress=end"), None);
    }

    #[test]
    fn decode_error_lines() {
        assert!(is_decode_error_line("[aac @ 0x1] Error decoding AAC frame header."));
        assert!(is_decode_error_line("Invalid data found when processing input"));
        assert!(is_decode_error_line("[mov,mp4 @ 0x1] stream 0, offset 0x1234: partial file"));
        assert!(!is_decode_error_line("[mov,mp4 @ 0x1] Referenced QT chapter track not found"));
    }

    #[test]
    fn probe_json_parsing() {
        let json = r#"{
            "streams": [
                {"codec_type": "video", "codec_name": "mjpeg"},
                {"codec_type": "audio", "codec_name": "aac", "sample_rate": "44100",
                 "channels": 2, "bit_rate": "125588"}
            ],
            "chapters": [
                {"start_time": "0.000000", "end_time": "12.500000", "tags": {"title": "Opening"}},
                {"start_time": "12.500000", "end_time": "12.500000", "tags": {"title": "Empty"}},
                {"start_time": "12.500000", "end_time": "30.000000"}
            ],
            "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "30.000000", "bit_rate": "128000"}
        }"#;
        let info = parse_probe_json(json).unwrap();
        assert!(info.is_mp4());
        assert!(!info.is_mp3());
        assert_eq!(info.duration_ms(), Some(30_000));
        assert_eq!(info.audio_bit_rate(), Some(125_588));
        let audio = info.audio.unwrap();
        assert_eq!(audio.codec, "aac");
        assert_eq!(audio.sample_rate, Some(44_100));
        assert_eq!(audio.channels, Some(2));
        assert_eq!(
            info.chapters,
            vec![
                Chapter { title: "Opening".into(), start_ms: 0, end_ms: 12_500 },
                Chapter { title: "Chapter 2".into(), start_ms: 12_500, end_ms: 30_000 },
            ]
        );
    }

    #[test]
    fn failure_messages_are_classified() {
        let outcome = |tail: &str| FfmpegOutcome {
            success: false,
            stderr_tail: tail.to_string(),
            decode_errors: 0,
            out_time_secs: None,
        };
        assert_eq!(
            ffmpeg_failure("Joining", &outcome("Error writing: No space left on device"), false).kind,
            FailureKind::DiskFull
        );
        assert_eq!(
            ffmpeg_failure("Decrypting", &outcome("x.aaxc: Invalid data found when processing input"), true).kind,
            FailureKind::Validation
        );
        assert_eq!(
            ffmpeg_failure("Encoding", &outcome("Unknown encoder 'libmp3lame'"), false).kind,
            FailureKind::Ffmpeg
        );
    }
}
