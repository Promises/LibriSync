use axum::extract::{Path, State};
use axum::response::Response;
use axum::Json;
use serde::Deserialize;

use super::{comma_list, no_content, Page, Paging};
use crate::error::{ApiError, ApiResult, JsonBody, QueryParams};
use crate::history::{self, HistoryItem, EVENTS};
use crate::queue::{self, Priority, QueueItem, QueueRow, QueueState};
use crate::state::AppState;
use crate::time;

pub async fn list(State(app): State<AppState>) -> ApiResult<Json<Vec<QueueItem>>> {
    let rows = queue::list_active(&app.db).await?;
    Ok(Json(rows.iter().map(|r| queue::to_item(r, &app.queue)).collect()))
}

async fn row_or_404(app: &AppState, id: &str) -> ApiResult<QueueRow> {
    let id: i64 = id.parse().map_err(|_| ApiError::not_found("Queue item not found"))?;
    queue::get(&app.db, id).await?.ok_or_else(|| ApiError::not_found("Queue item not found"))
}

pub async fn cancel(State(app): State<AppState>, Path(id): Path<String>) -> ApiResult<Response> {
    let row = row_or_404(&app, &id).await?;
    match row.state() {
        QueueState::Planning | QueueState::Running if app.queue.cancel(row.id) => {
            // The worker records the cancellation once the pipeline has stopped;
            // partial files stay in the work dir for a retry.
        }
        QueueState::Completed | QueueState::Cancelled => {}
        _ => {
            sqlx::query("UPDATE queue SET state = 'cancelled', finished_at = ?, waiting_reason = NULL WHERE id = ?")
                .bind(time::now_str())
                .bind(row.id)
                .execute(&app.db)
                .await?;
            queue::publish_id(&app, row.id).await;
        }
    }
    Ok(no_content())
}

pub async fn retry(State(app): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<QueueItem>> {
    let row = row_or_404(&app, &id).await?;
    if !matches!(row.state(), QueueState::Failed | QueueState::Cancelled) {
        return Err(ApiError::conflict("Only failed or cancelled items can be retried"));
    }
    let result = sqlx::query(
        "UPDATE queue SET state = 'queued', attempts = 0, error = NULL, not_before = NULL, waiting_reason = NULL,
                started_at = NULL, finished_at = NULL WHERE id = ?",
    )
    .bind(row.id)
    .execute(&app.db)
    .await;
    match result {
        Ok(_) => {}
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(ApiError::conflict("This book is already in the queue"));
        }
        Err(e) => return Err(e.into()),
    }
    app.queue.wake();
    let row = row_or_404(&app, &id).await?;
    queue::publish(&app, &row);
    Ok(Json(queue::to_item(&row, &app.queue)))
}

#[derive(Deserialize)]
pub struct MoveBody {
    priority: Priority,
}

pub async fn move_item(State(app): State<AppState>, Path(id): Path<String>, JsonBody(b): JsonBody<MoveBody>) -> ApiResult<Json<QueueItem>> {
    let row = row_or_404(&app, &id).await?;
    if !matches!(row.state(), QueueState::Queued | QueueState::Waiting) {
        return Err(ApiError::conflict("Only items that haven't started can be moved"));
    }
    sqlx::query("UPDATE queue SET priority = ? WHERE id = ?")
        .bind(b.priority.as_i64())
        .bind(row.id)
        .execute(&app.db)
        .await?;
    app.queue.wake();
    let row = row_or_404(&app, &id).await?;
    queue::publish(&app, &row);
    Ok(Json(queue::to_item(&row, &app.queue)))
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    event: Option<String>,
    account: Option<String>,
    book: Option<String>,
    page: Option<i64>,
    page_size: Option<i64>,
}

pub async fn history(State(app): State<AppState>, QueryParams(q): QueryParams<HistoryQuery>) -> ApiResult<Json<Page<HistoryItem>>> {
    let (page, page_size) = Paging { page: q.page, page_size: q.page_size }.resolve()?;
    let events = comma_list(q.event.as_deref());
    if let Some(bad) = events.iter().find(|e| !EVENTS.contains(&e.as_str())) {
        return Err(ApiError::invalid_field("event", format!("unknown event '{bad}'")));
    }
    let (total, items) = history::list(
        &app.db,
        &events,
        q.account.as_deref().filter(|a| !a.is_empty()),
        q.book.as_deref().filter(|b| !b.is_empty()),
        page,
        page_size,
    )
    .await?;
    Ok(Json(Page { total, page, page_size, items }))
}
