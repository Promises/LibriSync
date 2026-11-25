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
//!
//! A partial is only resumed against the object it came from. When a download starts,
//! `<target>.part.meta` records the total length and the `ETag` / `Last-Modified` the
//! server sent; a resume sends `If-Range` with that validator, and the partial is
//! discarded if the server answers with the whole file, a different total length or a
//! different validator. A partial without its sidecar is never resumed.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use reqwest::header::{
    HeaderMap, HeaderName, HeaderValue, CONTENT_RANGE, ETAG, IF_RANGE, LAST_MODIFIED, RANGE,
};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
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

/// The sidecar describing which object a partial download belongs to.
pub(crate) fn partial_meta_path(partial: &Path) -> PathBuf {
    let mut name = partial.file_name().unwrap_or_default().to_os_string();
    name.push(".meta");
    partial.with_file_name(name)
}

/// What the server said about the object when a download started (`<name>.part.meta`).
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PartMeta {
    #[serde(default)]
    pub total: Option<u64>,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
}

impl PartMeta {
    fn from_headers(headers: &HeaderMap, total: Option<u64>) -> Self {
        let text = |name| {
            headers
                .get(name)
                .and_then(|v: &HeaderValue| v.to_str().ok())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };
        PartMeta { total, etag: text(ETAG), last_modified: text(LAST_MODIFIED) }
    }

    /// The validator for `If-Range`: a strong ETag, else `Last-Modified`. A weak ETag
    /// may not be used there (RFC 9110 §13.1.5); the server would always send the
    /// whole file.
    pub(crate) fn if_range(&self) -> Option<&str> {
        self.etag
            .as_deref()
            .filter(|tag| !tag.starts_with("W/"))
            .or(self.last_modified.as_deref())
    }

    /// Whether `seen` (from a resumed response) describes the same object. Only values
    /// both sides know are compared; the ETag decides when both have one.
    pub(crate) fn same_object(&self, seen: &PartMeta) -> bool {
        if let (Some(a), Some(b)) = (self.total, seen.total) {
            if a != b {
                return false;
            }
        }
        let weak = |tag: &str| tag.trim_start_matches("W/").to_string();
        match (&self.etag, &seen.etag) {
            (Some(a), Some(b)) => weak(a) == weak(b),
            _ => match (&self.last_modified, &seen.last_modified) {
                (Some(a), Some(b)) => a == b,
                _ => true,
            },
        }
    }
}

async fn read_meta(partial: &Path) -> Option<PartMeta> {
    let bytes = tokio::fs::read(partial_meta_path(partial)).await.ok()?;
    serde_json::from_slice(&bytes).ok()
}

async fn write_meta(partial: &Path, meta: &PartMeta) -> Result<(), PipelineError> {
    let path = partial_meta_path(partial);
    let json = serde_json::to_vec(meta).map_err(|e| {
        PipelineError::new(FailureKind::Io, format!("encoding {}: {e}", path.display()))
    })?;
    tokio::fs::write(&path, json)
        .await
        .map_err(|e| io_error(format_args!("writing {}", path.display()), e))
}

async fn remove_meta(partial: &Path) {
    let _ = tokio::fs::remove_file(partial_meta_path(partial)).await;
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
/// The URL is stripped from the message: download links are signed, and this message
/// ends up in queue errors, history and webhooks.
fn network_error(e: reqwest::Error) -> PipelineError {
    let e = e.without_url();
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
                remove_meta(&partial).await;
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

/// Empty a partial download and forget which object it belonged to.
async fn truncate(path: &Path) -> Result<(), PipelineError> {
    remove_meta(path).await;
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
    let mut offset = tokio::fs::metadata(partial).await.map(|m| m.len()).unwrap_or(0);
    let stored = if offset > 0 { read_meta(partial).await } else { None };
    if offset > 0 && stored.is_none() {
        // Nothing says which object these bytes came from, so they can't be trusted.
        log::warn!("pipeline: part {}: partial file has no record of its source; restarting", req.part);
        truncate(partial).await?;
        offset = 0;
    }
    let mut request = http.get(req.url).headers(headers.clone());
    if offset > 0 {
        request = request.header(RANGE, format!("bytes={offset}-"));
        if let Some(validator) = stored.as_ref().and_then(PartMeta::if_range) {
            if let Ok(value) = HeaderValue::from_str(validator) {
                request = request.header(IF_RANGE, value);
            }
        }
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
            let error = network_error(e);
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

    // What this response says about the object, to compare with the stored sidecar.
    let different_object = |total: Option<u64>| {
        let seen = PartMeta::from_headers(response.headers(), total);
        stored.as_ref().is_some_and(|stored| !stored.same_object(&seen))
    };

    // (write offset, expected total)
    let (start_at, total) = match status {
        StatusCode::PARTIAL_CONTENT if offset > 0 => match content_range {
            Some((Some(start), total)) if start == offset && different_object(total) => {
                log::warn!(
                    "pipeline: part {}: the file on the server changed since the partial \
                     download; restarting",
                    req.part
                );
                truncate(partial).await?;
                return Ok(Attempt::Retry {
                    error: PipelineError::new(
                        FailureKind::Http,
                        "the file on the server changed during the download",
                    ),
                    progressed: false,
                });
            }
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
            if matches!(content_range, Some((None, Some(total))) if total == offset)
                && !different_object(Some(offset))
            {
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
        // Record which object the partial will hold before the first byte lands in it.
        write_meta(partial, &PartMeta::from_headers(response.headers(), total)).await?;
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
            Ok(Some(Err(e))) => break Some(network_error(e)),
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
    fn partial_meta_sits_next_to_the_partial() {
        assert_eq!(
            partial_meta_path(&partial_path(Path::new("/w/001.m4a"))),
            Path::new("/w/001.m4a.part.meta")
        );
    }

    fn meta(total: Option<u64>, etag: Option<&str>, modified: Option<&str>) -> PartMeta {
        PartMeta {
            total,
            etag: etag.map(str::to_string),
            last_modified: modified.map(str::to_string),
        }
    }

    #[test]
    fn if_range_prefers_a_strong_etag() {
        let lm = "Wed, 21 Oct 2026 07:28:00 GMT";
        assert_eq!(meta(None, Some("\"a\""), Some(lm)).if_range(), Some("\"a\""));
        assert_eq!(meta(None, Some("W/\"a\""), Some(lm)).if_range(), Some(lm));
        assert_eq!(meta(None, Some("W/\"a\""), None).if_range(), None);
        assert_eq!(meta(Some(5), None, None).if_range(), None);
    }

    #[test]
    fn same_object_compares_what_both_sides_know() {
        let stored = meta(Some(100), Some("\"a\""), Some("Mon"));
        assert!(stored.same_object(&meta(Some(100), Some("\"a\""), Some("Tue"))));
        assert!(stored.same_object(&meta(Some(100), Some("W/\"a\""), None)));
        assert!(stored.same_object(&meta(None, None, None)));
        assert!(!stored.same_object(&meta(Some(101), Some("\"a\""), None)));
        assert!(!stored.same_object(&meta(Some(100), Some("\"b\""), None)));
        let no_tag = meta(Some(100), None, Some("Mon"));
        assert!(!no_tag.same_object(&meta(Some(100), None, Some("Tue"))));
        assert!(no_tag.same_object(&meta(Some(100), Some("\"x\""), Some("Mon"))));
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
