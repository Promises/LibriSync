//! Throttled progress reporting.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::{Progress, ProgressFn, Stage};

/// Minimum gap between throttled reports.
pub(crate) const REPORT_INTERVAL: Duration = Duration::from_secs(1);

/// Wraps the caller's [`ProgressFn`]: stage changes go out at once, in-stage updates at
/// most once per [`REPORT_INTERVAL`].
pub(crate) struct Reporter {
    callback: ProgressFn,
    parts: usize,
    last: Mutex<Option<Instant>>,
}

impl Reporter {
    pub(crate) fn new(callback: ProgressFn, parts: usize) -> Self {
        Self { callback, parts: parts.max(1), last: Mutex::new(None) }
    }

    fn due(&self, force: bool) -> bool {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if force || last.is_none_or(|t| now.duration_since(t) >= REPORT_INTERVAL) {
            *last = Some(now);
            true
        } else {
            false
        }
    }

    /// Report a stage (always delivered).
    pub(crate) fn stage(&self, stage: Stage, fraction: Option<f32>) {
        self.emit(stage, fraction, true);
    }

    /// Report progress within a stage (throttled).
    pub(crate) fn fraction(&self, stage: Stage, fraction: f32) {
        self.emit(stage, Some(fraction), false);
    }

    fn emit(&self, stage: Stage, fraction: Option<f32>, force: bool) {
        if self.due(force) {
            (self.callback)(Progress {
                stage,
                part: 1,
                parts: self.parts,
                bytes_done: 0,
                bytes_total: None,
                bytes_per_sec: None,
                fraction: fraction.map(|f| f.clamp(0.0, 1.0)),
            });
        }
    }

    /// Report download progress for `part` (1-based). `force` bypasses the throttle
    /// (start and end of a part).
    pub(crate) fn download(
        &self,
        part: usize,
        bytes_done: u64,
        bytes_total: Option<u64>,
        bytes_per_sec: Option<u64>,
        force: bool,
    ) {
        if self.due(force) {
            (self.callback)(Progress {
                stage: Stage::Downloading,
                part,
                parts: self.parts,
                bytes_done,
                bytes_total,
                bytes_per_sec,
                fraction: bytes_total
                    .filter(|t| *t > 0)
                    .map(|t| (bytes_done as f64 / t as f64).clamp(0.0, 1.0) as f32),
            });
        }
    }
}
