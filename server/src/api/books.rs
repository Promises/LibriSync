use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{comma_list, OptionalJson, Page, Paging};
use crate::backfill::{self, Estimate};
use crate::books::{self, BookQuery, BookView, Sort, STATUSES};
use crate::covers;
use crate::error::{ApiError, ApiResult, JsonBody, QueryParams};
use crate::history::{self, HistoryItem};
use crate::queue::{self, EnqueueError, Priority, QueueItem};
use crate::state::AppState;
use crate::time;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListQuery {
    search: Option<String>,
    provider: Option<String>,
    account: Option<String>,
    status: Option<String>,
    sort: Option<String>,
    dir: Option<String>,
    page: Option<i64>,
    page_size: Option<i64>,
}

fn book_query(q: ListQuery) -> Result<BookQuery, ApiError> {
    let (page, page_size) = Paging { page: q.page, page_size: q.page_size }.resolve()?;
    let statuses = comma_list(q.status.as_deref());
    if let Some(bad) = statuses.iter().find(|s| !STATUSES.contains(&s.as_str())) {
        return Err(ApiError::invalid_field("status", format!("unknown status '{bad}'")));
    }
    let sort = match q.sort.as_deref() {
        None => Sort::Added,
        Some(s) => Sort::parse(s).ok_or_else(|| ApiError::invalid_field("sort", "must be title, author, added, purchased or released"))?,
    };
    let desc = match q.dir.as_deref() {
        None => true,
        Some("desc") => true,
        Some("asc") => false,
        Some(_) => return Err(ApiError::invalid_field("dir", "must be asc or desc")),
    };
    if let Some(p) = q.provider.as_deref() {
        if !matches!(p, "audible" | "librofm" | "chirp") {
            return Err(ApiError::invalid_field("provider", "must be audible, librofm or chirp"));
        }
    }
    Ok(BookQuery {
        search: q.search.filter(|s| !s.trim().is_empty()),
        provider: q.provider,
        account: q.account.filter(|a| !a.is_empty()),
        statuses,
        sort,
        desc,
        page,
        page_size,
        id: None,
    })
}

pub async fn list(State(app): State<AppState>, QueryParams(q): QueryParams<ListQuery>) -> ApiResult<Json<Page<BookView>>> {
    let q = book_query(q)?;
    let (total, items) = books::list(&app.db, &q).await?;
    Ok(Json(Page { total, page: q.page, page_size: q.page_size, items }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WantedPage {
    #[serde(flatten)]
    page: Page<BookView>,
    estimate: Estimate,
}

pub async fn wanted(State(app): State<AppState>, QueryParams(mut q): QueryParams<ListQuery>) -> ApiResult<Json<WantedPage>> {
    q.status = Some("wanted".into());
    let q = book_query(q)?;
    let (total, items) = books::list(&app.db, &q).await?;
    let estimate = backfill::estimate(&app).await?;
    Ok(Json(WantedPage { page: Page { total, page: q.page, page_size: q.page_size, items }, estimate }))
}

#[derive(Serialize)]
pub struct BookDetail {
    #[serde(flatten)]
    book: BookView,
    description: String,
    history: Vec<HistoryItem>,
}

async fn view_or_404(app: &AppState, id: &str) -> ApiResult<BookView> {
    books::get(&app.db, id).await?.ok_or_else(|| ApiError::not_found("Book not found"))
}

pub async fn get(State(app): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<BookDetail>> {
    let book = view_or_404(&app, &id).await?;
    let description = books::description(&app.db, &id).await?;
    let (_, history) = history::list(&app.db, &[], None, Some(&id), 1, 50).await?;
    Ok(Json(BookDetail { book, description, history }))
}

#[derive(Default, Deserialize)]
pub struct DownloadBody {
    #[serde(default)]
    force: bool,
}

fn enqueue_error(e: EnqueueError) -> ApiError {
    match e {
        EnqueueError::NotFound => ApiError::not_found("Book not found"),
        EnqueueError::NoAccount => ApiError::conflict("No signed-in account owns this book"),
        EnqueueError::Cooldown(until) => ApiError::account_cooldown(format!(
            "Audible is throttling this account until {}; retrying now is believed to extend it. Send force to try anyway.",
            time::local_label(until)
        )),
        EnqueueError::AlreadyDownloaded => ApiError::conflict("Already downloaded"),
        EnqueueError::Skipped => ApiError::conflict("The book is skipped"),
        EnqueueError::Backoff(until) => ApiError::conflict(format!("Licence backoff until {}", time::local_label(until))),
        EnqueueError::Db(e) => e.into(),
    }
}

pub async fn download(
    State(app): State<AppState>,
    Path(id): Path<String>,
    OptionalJson(body): OptionalJson<DownloadBody>,
) -> ApiResult<Json<QueueItem>> {
    let row = queue::enqueue(&app, &id, Priority::Manual, body.force).await.map_err(enqueue_error)?;
    Ok(Json(queue::to_item(&row, &app.queue)))
}

#[derive(Deserialize)]
pub struct BulkBody {
    ids: Vec<String>,
}

#[derive(Serialize)]
pub struct Skipped {
    id: String,
    reason: String,
}

#[derive(Serialize)]
pub struct BulkResult {
    queued: usize,
    skipped: Vec<Skipped>,
}

pub async fn download_many(State(app): State<AppState>, JsonBody(b): JsonBody<BulkBody>) -> ApiResult<Json<BulkResult>> {
    if b.ids.len() > 10_000 {
        return Err(ApiError::invalid_field("ids", "at most 10000 ids per request"));
    }
    let mut queued = 0;
    let mut skipped = Vec::new();
    for id in b.ids {
        match queue::enqueue(&app, &id, Priority::Manual, false).await {
            Ok(_) => queued += 1,
            Err(EnqueueError::Db(e)) => return Err(e.into()),
            Err(e) => skipped.push(Skipped { id, reason: e.reason() }),
        }
    }
    Ok(Json(BulkResult { queued, skipped }))
}

/// Remove not-yet-started (or failed) queue items for a book.
async fn drop_pending(app: &AppState, id: &str) -> ApiResult<()> {
    let removed: Vec<i64> =
        sqlx::query_scalar("DELETE FROM queue WHERE book_id = ? AND state IN ('queued','waiting','failed') RETURNING id")
            .bind(id)
            .fetch_all(&app.db)
            .await?;
    for qid in removed {
        app.hub.publish("queueRemoved", &json!({ "id": qid.to_string() }));
    }
    Ok(())
}

pub async fn skip(State(app): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<BookView>> {
    view_or_404(&app, &id).await?;
    sqlx::query("INSERT INTO skips (book_id, created_at) VALUES (?, ?) ON CONFLICT(book_id) DO NOTHING")
        .bind(&id)
        .bind(time::now_str())
        .execute(&app.db)
        .await?;
    drop_pending(&app, &id).await?;
    Ok(Json(view_or_404(&app, &id).await?))
}

pub async fn unskip(State(app): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<BookView>> {
    view_or_404(&app, &id).await?;
    sqlx::query("DELETE FROM skips WHERE book_id = ?").bind(&id).execute(&app.db).await?;
    Ok(Json(view_or_404(&app, &id).await?))
}

pub async fn forget(State(app): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<BookView>> {
    view_or_404(&app, &id).await?;
    sqlx::query("DELETE FROM downloads WHERE book_id = ?").bind(&id).execute(&app.db).await?;
    let removed: Vec<i64> = sqlx::query_scalar("DELETE FROM queue WHERE book_id = ? AND state = 'failed' RETURNING id")
        .bind(&id)
        .fetch_all(&app.db)
        .await?;
    for qid in removed {
        app.hub.publish("queueRemoved", &json!({ "id": qid.to_string() }));
    }
    Ok(Json(view_or_404(&app, &id).await?))
}

pub async fn cover(State(app): State<AppState>, Path(id): Path<String>) -> ApiResult<Response> {
    let (bytes, ct) = covers::get(&app, &id).await?;
    Ok(([(header::CONTENT_TYPE, ct), (header::CACHE_CONTROL, "private, max-age=86400")], bytes).into_response())
}
