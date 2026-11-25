//! Resumable single-file HTTP download.
//!
//! Replaces `download::PersistentDownloadManager` for the pipeline, fixing the bugs the
//! Android engine lives with (`persistent_manager.rs`): a cancel no longer looks like a
//! completed download, the final length is checked against `Content-Length` /
//! `Content-Range`, a `200` answer to a `Range` request restarts from zero instead of
//! appending a second copy, stalled sockets time out, and transient failures are retried
//! in-run with backoff.
//!
//! Layout: bytes stream into `<target>.part`; only a verified-complete file is renamed
//! to `<target>`. A later run finds either the finished file (reused) or the partial
//! (resumed with `Range`).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_RANGE, RANGE};
use reqwest::StatusCode;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio_util::sync::CancellationToken;

use crate::providers::Headers;

use super::fsutil::io_error;
use super::progress::Reporter;
use super::tools::cancelled;
use super::{FailureKind, PipelineError};

/// Give up on a response whose headers or next chunk take longer than this.
pub(crate) const STALL_TIMEOUT: Duration = Duration::from_secs(60);
/// Consecutive failed attempts (no bytes gained) before giving up.
const MAX_ATTEMPTS: u32 = 5;
/// First retry delay; doubles per consecutive failure, capped at [`MAX_BACKOFF`].
const BASE_BACKOFF: Duration = Duration::from_secs(2);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const WRITE_BUFFER: usize = 256 * 1024;

/// Where a part's bytes came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fetched {
    /// A complete file from an earlier run was found; nothing was downloaded.
    Reused,
    /// Downloaded (or finished resuming) in this run.
    Downloaded,
}

/// One part to fetch.
pub(crate) struct PartRequest<'a> {
    pub url: &'a str,
    pub headers: &'a Headers,
    /// Final path of the complete file.
    pub target: &'a Path,
    /// 1-based part number, for progress.
    pub part: usize,
}

/// The partial-download path for `target`.
pub(crate) fn partial_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    target.with_file_name(name)
}

/// Outcome of one HTTP attempt.
enum Attempt {
    Complete,
    /// Worth another try. `progressed` = bytes were gained, so the failure counter resets.
    Retry { error: PipelineError, progressed: bool },
}

/// Parse `Content-Range: bytes <start>-<end>/<total|*>` (or `bytes */<total>` on 416).
/// Returns `(start, total)`; `start` is `None` for the unsatisfied form.
pub(crate) fn parse_content_range(value: &str) -> Option<(Option<u64>, Option<u64>)> {
    let rest = value.trim().strip_prefix("bytes")?.trim_start();
    let (range, total) = rest.split_once('/')?;
    let total = match total.trim() {
        "*" => None,
        t => Some(t.parse().ok()?),
    };
    let start = match range.trim() {
        "*" => None,
        r => Some(r.split_once('-')?.0.trim().parse().ok()?),
    };
    Some((start, total))
}

fn header_map(headers: &Headers) -> Result<HeaderMap, PipelineError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|e| {
            PipelineError::new(FailureKind::InvalidPlan, format!("bad header name {name:?}: {e}"))
        })?;
        let value = HeaderValue::from_str(value).map_err(|e| {
            PipelineError::new(FailureKind::InvalidPlan, format!("bad value for header {name}: {e}"))
        })?;
        map.insert(name, value);
    }
    Ok(map)
}

/// Classify a reqwest error. Everything that isn't a malformed request is transient.
fn network_error(e: &reqwest::Error) -> PipelineError {
    if e.is_builder() {
        PipelineError::new(FailureKind::InvalidPlan, format!("bad download URL: {e}"))
    } else {
        PipelineError::new(FailureKind::Network, format!("network error: {e}"))
    }
}

fn status_error(status: StatusCode) -> PipelineError {
    match status.as_u16() {
        401 | 403 | 410 => PipelineError::new(
            FailureKind::Expired,
            format!("the download link was refused (HTTP {status}); it has probably expired"),
        ),
        _ => PipelineError::new(FailureKind::Http, format!("download failed with HTTP {status}")),
    }
}

fn is_transient_status(status: StatusCode) -> bool {
    status.is_server_error()
        || status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
}

/// Download one part with resume, verification and in-run retries.
pub(crate) async fn fetch_part(
    http: &reqwest::Client,
    req: &PartRequest<'_>,
    reporter: &Reporter,
    cancel: &CancellationToken,
) -> Result<Fetched, PipelineError> {
    if let Ok(meta) = tokio::fs::metadata(req.target).await {
        if meta.is_file() {
            log::info!(
                "pipeline: reusing complete part {} ({} bytes)",
                req.target.display(),
                meta.len()
            );
            reporter.download(req.part, meta.len(), Some(meta.len()), None, true);
            return Ok(Fetched::Reused);
        }
    }
    let headers = header_map(req.headers)?;
    let partial = partial_path(req.target);
    let mut failures = 0u32;
    loop {
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        match attempt(http, req, &headers, &partial, reporter, cancel).await? {
            Attempt::Complete => {
                tokio::fs::rename(&partial, req.target).await.map_err(|e| {
                    io_error(format_args!("finishing {}", req.target.display()), e)
                })?;
                return Ok(Fetched::Downloaded);
            }
            Attempt::Retry { error, progressed } => {
                failures = if progressed { 1 } else { failures + 1 };
                if failures >= MAX_ATTEMPTS {
                    return Err(error);
                }
                let delay = BASE_BACKOFF
                    .saturating_mul(1 << (failures - 1).min(8))
                    .min(MAX_BACKOFF);
                log::warn!(
                    "pipeline: part {} attempt failed ({}); retrying in {delay:?}",
                    req.part,
                    error.message
                );
                tokio::select! {
                    _ = cancel.cancelled() => return Err(cancelled()),
                    _ = tokio::time::sleep(delay) => {}
                }
            }
        }
    }
}

async fn truncate(path: &Path) -> Result<(), PipelineError> {
    File::create(path)
        .await
        .map(|_| ())
        .map_err(|e| io_error(format_args!("resetting {}", path.display()), e))
}

/// One request: send, interpret the status, stream the body onto the partial file.
/// `Err` is final (cancel, expired, disk full, bad plan); `Attempt::Retry` is not.
async fn attempt(
    http: &reqwest::Client,
    req: &PartRequest<'_>,
    headers: &HeaderMap,
    partial: &Path,
    reporter: &Reporter,
    cancel: &CancellationToken,
) -> Result<Attempt, PipelineError> {
    let offset = tokio::fs::metadata(partial).await.map(|m| m.len()).unwrap_or(0);
    let mut request = http.get(req.url).headers(headers.clone());
    if offset > 0 {
        request = request.header(RANGE, format!("bytes={offset}-"));
    }
    let response = tokio::select! {
        _ = cancel.cancelled() => return Err(cancelled()),
        r = tokio::time::timeout(STALL_TIMEOUT, request.send()) => r,
    };
    let response = match response {
        Err(_) => {
            return Ok(Attempt::Retry {
                error: PipelineError::new(FailureKind::Network, "no response from the server"),
                progressed: false,
            })
        }
        Ok(Err(e)) => {
            let error = network_error(&e);
            if error.kind == FailureKind::InvalidPlan {
                return Err(error);
            }
            return Ok(Attempt::Retry { error, progressed: false });
        }
        Ok(Ok(r)) => r,
    };

    let status = response.status();
    let content_range = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_content_range);

    // (write offset, expected total)
    let (start_at, total) = match status {
        StatusCode::PARTIAL_CONTENT if offset > 0 => match content_range {
            Some((Some(start), total)) if start == offset => (offset, total),
            _ => {
                log::warn!(
                    "pipeline: part {}: server resumed at the wrong offset; restarting",
                    req.part
                );
                truncate(partial).await?;
                return Ok(Attempt::Retry {
                    error: PipelineError::new(FailureKind::Http, "server sent an unexpected range"),
                    progressed: false,
                });
            }
        },
        StatusCode::RANGE_NOT_SATISFIABLE if offset > 0 => {
            if matches!(content_range, Some((None, Some(total))) if total == offset) {
                // The partial already holds every byte.
                reporter.download(req.part, offset, Some(offset), None, true);
                return Ok(Attempt::Complete);
            }
            log::warn!("pipeline: part {}: partial file is unusable; restarting", req.part);
            truncate(partial).await?;
            return Ok(Attempt::Retry {
                error: PipelineError::new(FailureKind::Http, "server refused the resume range"),
                progressed: false,
            });
        }
        s if s.is_success() => {
            if offset > 0 {
                // A 200 to a Range request is the whole file: appending it would corrupt
                // the partial (the Android manager's bug), so start over.
                log::info!(
                    "pipeline: part {}: server ignored the resume range; restarting from zero",
                    req.part
                );
            }
            (0, response.content_length())
        }
        s if is_transient_status(s) => {
            return Ok(Attempt::Retry { error: status_error(s), progressed: false });
        }
        s => return Err(status_error(s)),
    };

    let file = if start_at == 0 {
        File::create(partial).await
    } else {
        OpenOptions::new().append(true).open(partial).await
    }
    .map_err(|e| io_error(format_args!("opening {}", partial.display()), e))?;
    let mut writer = BufWriter::with_capacity(WRITE_BUFFER, file);

    let mut done = start_at;
    let initial = start_at;
    let mut window_start = Instant::now();
    let mut window_bytes = 0u64;
    let mut rate: Option<u64> = None;
    reporter.download(req.part, done, total, None, true);

    let mut stream = response.bytes_stream();
    let stream_error: Option<PipelineError> = loop {
        let next = tokio::select! {
            _ = cancel.cancelled() => {
                // Keep what we have for a later resume.
                let _ = writer.flush().await;
                return Err(cancelled());
            }
            next = tokio::time::timeout(STALL_TIMEOUT, stream.next()) => next,
        };
        match next {
            Err(_) => {
                break Some(PipelineError::new(
                    FailureKind::Network,
                    format!("download stalled for {}s", STALL_TIMEOUT.as_secs()),
                ))
            }
            Ok(None) => break None,
            Ok(Some(Err(e))) => break Some(network_error(&e)),
            Ok(Some(Ok(chunk))) => {
                if let Err(e) = writer.write_all(&chunk).await {
                    let _ = writer.flush().await;
                    return Err(io_error(format_args!("writing {}", partial.display()), e));
                }
                done += chunk.len() as u64;
                window_bytes += chunk.len() as u64;
                let elapsed = window_start.elapsed();
                if elapsed >= super::progress::REPORT_INTERVAL {
                    rate = Some((window_bytes as f64 / elapsed.as_secs_f64()) as u64);
                    window_start = Instant::now();
                    window_bytes = 0;
                }
                reporter.download(req.part, done, total, rate, false);
            }
        }
    };
    writer
        .flush()
        .await
        .map_err(|e| io_error(format_args!("writing {}", partial.display()), e))?;
    drop(writer);
    let progressed = done > initial;

    if let Some(error) = stream_error {
        return Ok(Attempt::Retry { error, progressed });
    }
    let on_disk = tokio::fs::metadata(partial).await.map(|m| m.len()).unwrap_or(0);
    match total {
        Some(total) if on_disk < total => Ok(Attempt::Retry {
            error: PipelineError::new(
                FailureKind::Network,
                format!("connection closed early ({on_disk} of {total} bytes)"),
            ),
            progressed,
        }),
        Some(total) if on_disk > total => {
            truncate(partial).await?;
            Ok(Attempt::Retry {
                error: PipelineError::new(
                    FailureKind::Http,
                    format!("received {on_disk} bytes but the server announced {total}"),
                ),
                progressed: false,
            })
        }
        _ => {
            reporter.download(req.part, on_disk, Some(on_disk), rate, true);
            Ok(Attempt::Complete)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_range_parsing() {
        assert_eq!(parse_content_range("bytes 100-199/1000"), Some((Some(100), Some(1000))));
        assert_eq!(parse_content_range("bytes 100-199/*"), Some((Some(100), None)));
        assert_eq!(parse_content_range("bytes */1000"), Some((None, Some(1000))));
        assert_eq!(parse_content_range("items 1-2/3"), None);
        assert_eq!(parse_content_range("bytes x-y/z"), None);
    }

    #[test]
    fn partial_path_appends_part() {
        assert_eq!(partial_path(Path::new("/w/001.m4a")), Path::new("/w/001.m4a.part"));
    }

    #[test]
    fn status_mapping() {
        assert_eq!(status_error(StatusCode::FORBIDDEN).kind, FailureKind::Expired);
        assert_eq!(status_error(StatusCode::GONE).kind, FailureKind::Expired);
        assert_eq!(status_error(StatusCode::UNAUTHORIZED).kind, FailureKind::Expired);
        assert_eq!(status_error(StatusCode::NOT_FOUND).kind, FailureKind::Http);
        assert!(is_transient_status(StatusCode::BAD_GATEWAY));
        assert!(is_transient_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(!is_transient_status(StatusCode::NOT_FOUND));
    }
}
