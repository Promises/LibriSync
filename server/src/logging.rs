//! Tracing setup: stdout, daily-rolling files in `<config>/logs`, and an in-memory ring
//! buffer that backs `GET /api/v1/log`. The level can be changed at runtime from the UI
//! unless `LIBRISYNC_LOG` pins it.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::io::IsTerminal;
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{reload, EnvFilter, Layer, Registry};

const RING_CAPACITY: usize = 5000;
const LOG_FILES_KEPT: usize = 14;

#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    pub time: String,
    pub level: &'static str,
    pub target: String,
    pub message: String,
}

/// The most recent log lines, oldest first.
#[derive(Clone, Default)]
pub struct LogBuffer(Arc<Mutex<VecDeque<LogLine>>>);

impl LogBuffer {
    fn push(&self, line: LogLine) {
        if let Ok(mut buf) = self.0.lock() {
            if buf.len() >= RING_CAPACITY {
                buf.pop_front();
            }
            buf.push_back(line);
        }
    }

    /// The last `lines` entries at `min_level` or more severe, newest last.
    pub fn tail(&self, lines: usize, min_level: Level) -> Vec<LogLine> {
        let Ok(buf) = self.0.lock() else { return Vec::new() };
        let mut out: Vec<LogLine> = buf
            .iter()
            .rev()
            .filter(|l| level_of(l.level) <= min_level)
            .take(lines)
            .cloned()
            .collect();
        out.reverse();
        out
    }
}

fn level_str(level: &Level) -> &'static str {
    match *level {
        Level::TRACE => "trace",
        Level::DEBUG => "debug",
        Level::INFO => "info",
        Level::WARN => "warn",
        Level::ERROR => "error",
    }
}

/// `tracing::Level` orders TRACE > DEBUG > … > ERROR (more verbose = greater).
fn level_of(s: &str) -> Level {
    match s {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    }
}

pub fn parse_level(s: &str) -> Option<Level> {
    match s {
        "trace" | "debug" | "info" | "warn" | "error" => Some(level_of(s)),
        _ => None,
    }
}

struct RingLayer(LogBuffer);

#[derive(Default)]
struct MessageVisitor {
    message: String,
    fields: String,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            let _ = write!(self.fields, " {}={value:?}", field.name());
        }
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            let _ = write!(self.fields, " {}={value}", field.name());
        }
    }
}

impl<S: Subscriber> Layer<S> for RingLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut v = MessageVisitor::default();
        event.record(&mut v);
        let meta = event.metadata();
        self.0.push(LogLine {
            time: crate::time::now_str(),
            level: level_str(meta.level()),
            target: meta.target().to_string(),
            message: format!("{}{}", v.message, v.fields),
        });
    }
}

/// Changes the active log filter.
pub type LevelSetter = Arc<dyn Fn(&str) + Send + Sync>;

pub struct Logging {
    pub buffer: LogBuffer,
    pub set_level: LevelSetter,
    _file_guard: Option<WorkerGuard>,
}

fn filter_for(level: &str) -> EnvFilter {
    // Our crates at the chosen level; chatty dependencies held back.
    EnvFilter::try_new(format!(
        "{level},librisync_server={level},rust_core={level},sqlx=warn,hyper=warn,reqwest=warn,tower_http=warn"
    ))
    .unwrap_or_else(|_| EnvFilter::new("info"))
}

/// Install the global subscriber. `level_override` (from `LIBRISYNC_LOG`) may be a full
/// `EnvFilter` directive; when set, the UI's level setting is ignored.
pub fn init(logs_dir: &Path, level_override: Option<&str>, initial_level: &str) -> Logging {
    let buffer = LogBuffer::default();
    let filter = match level_override {
        Some(directive) => EnvFilter::try_new(directive).unwrap_or_else(|_| filter_for("info")),
        None => filter_for(initial_level),
    };
    let (filter_layer, handle) = reload::Layer::<EnvFilter, Registry>::new(filter);

    let (file_layer, guard) = match std::fs::create_dir_all(logs_dir).ok().and_then(|_| {
        RollingFileAppender::builder()
            .rotation(Rotation::DAILY)
            .filename_prefix("librisync")
            .filename_suffix("log")
            .max_log_files(LOG_FILES_KEPT)
            .build(logs_dir)
            .ok()
    }) {
        Some(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            (
                Some(tracing_subscriber::fmt::layer().with_ansi(false).with_writer(writer)),
                Some(guard),
            )
        }
        None => (None, None),
    };

    let registry = tracing_subscriber::registry()
        .with(filter_layer)
        .with(tracing_subscriber::fmt::layer().with_ansi(std::io::stdout().is_terminal()))
        .with(file_layer)
        .with(RingLayer(buffer.clone()));
    // A second init (tests) is harmless: keep the first subscriber.
    let _ = registry.try_init();

    let pinned = level_override.is_some();
    let set_level: LevelSetter = Arc::new(move |level: &str| {
        if pinned {
            return;
        }
        let _ = handle.reload(filter_for(level));
    });

    if guard.is_none() {
        tracing::warn!("Could not open the log directory {}; logging to stdout only", logs_dir.display());
    }

    Logging { buffer, set_level, _file_guard: guard }
}

/// Route panics through tracing so they land in the log files and the UI log, not
/// just stderr.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".to_string());
        tracing::error!(target: "panic", "{payload} at {location}");
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_filters_by_severity_and_keeps_order() {
        let buf = LogBuffer::default();
        for (i, level) in ["debug", "info", "warn", "error", "info"].iter().enumerate() {
            buf.push(LogLine {
                time: String::new(),
                level,
                target: "t".into(),
                message: i.to_string(),
            });
        }
        let lines: Vec<String> = buf.tail(10, Level::WARN).into_iter().map(|l| l.message).collect();
        assert_eq!(lines, vec!["2", "3"]);
        let last_two: Vec<String> = buf.tail(2, Level::TRACE).into_iter().map(|l| l.message).collect();
        assert_eq!(last_two, vec!["3", "4"]);
    }
}
