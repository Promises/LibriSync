use std::convert::Infallible;

use axum::extract::{Path, State};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::Response;
use axum::Json;
use futures_util::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::BroadcastStream;

use super::no_content;
use crate::error::{ApiError, ApiResult, QueryParams};
use crate::health::{self, HealthItem};
use crate::logging::{self, LogLine};
use crate::scheduler::{self, ScheduledTask, TaskName};
use crate::state::AppState;
use crate::time;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemStatus {
    version: &'static str,
    start_time: String,
    config_dir: String,
    library_dir: String,
    ffmpeg_version: Option<String>,
    is_docker: bool,
    os: String,
    accounts: i64,
    books: i64,
    downloaded: i64,
    queued: i64,
}

fn is_docker() -> bool {
    std::path::Path::new("/.dockerenv").exists()
        || std::fs::read_to_string("/proc/1/cgroup").is_ok_and(|s| s.contains("docker") || s.contains("containerd"))
}

pub async fn status(State(app): State<AppState>) -> ApiResult<Json<SystemStatus>> {
    let (accounts, books, downloaded, queued): (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM core.Accounts),
                (SELECT COUNT(*) FROM core.Books b WHERE b.content_type != 4 AND EXISTS (
                    SELECT 1 FROM core.BookAccounts ba JOIN core.Accounts a ON a.account_id = ba.account
                    WHERE ba.book_id = b.book_id AND ba.is_deleted = 0)),
                (SELECT COUNT(*) FROM downloads),
                (SELECT COUNT(*) FROM queue WHERE state IN ('queued','waiting','planning','running'))",
    )
    .fetch_one(&app.db)
    .await?;
    Ok(Json(SystemStatus {
        version: env!("CARGO_PKG_VERSION"),
        start_time: time::fmt(app.started),
        config_dir: app.cfg.config_dir.display().to_string(),
        library_dir: app.cfg.library_dir.display().to_string(),
        ffmpeg_version: app.tools_status().and_then(Result::ok),
        is_docker: is_docker(),
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        accounts,
        books,
        downloaded,
        queued,
    }))
}

pub async fn health(State(app): State<AppState>) -> Json<Vec<HealthItem>> {
    Json(health::check(&app).await)
}

pub async fn tasks(State(app): State<AppState>) -> Json<Vec<ScheduledTask>> {
    Json(scheduler::list(&app).await)
}

pub async fn run_task(State(app): State<AppState>, Path(name): Path<String>) -> ApiResult<Response> {
    let task = TaskName::parse(&name).ok_or_else(|| ApiError::not_found(format!("No task named '{name}'")))?;
    // Already running counts as success: the work the caller asked for is under way.
    app.tasks.trigger(task);
    Ok(no_content())
}

#[derive(Deserialize)]
pub struct LogQuery {
    lines: Option<usize>,
    level: Option<String>,
}

#[derive(Serialize)]
pub struct LogResponse {
    lines: Vec<LogLine>,
}

pub async fn log(State(app): State<AppState>, QueryParams(q): QueryParams<LogQuery>) -> ApiResult<Json<LogResponse>> {
    let level = match q.level.as_deref() {
        None => tracing::Level::INFO,
        Some(l) => logging::parse_level(l).ok_or_else(|| ApiError::invalid_field("level", "must be trace, debug, info, warn or error"))?,
    };
    let lines = q.lines.unwrap_or(500).clamp(1, 5000);
    Ok(Json(LogResponse { lines: app.logs.tail(lines, level) }))
}

pub async fn events(State(app): State<AppState>) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let stream = BroadcastStream::new(app.hub.subscribe())
        // A client too slow to keep up skips what it missed rather than erroring out.
        .filter_map(|msg| async move { msg.ok().map(|e| Ok(SseEvent::default().event(e.name).data(e.data))) })
        .take_until(app.shutdown.clone().cancelled_owned());
    Sse::new(stream).keep_alive(KeepAlive::default())
}
