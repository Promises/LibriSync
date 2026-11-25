//! History events (Activity → History), and the single place that fans an event out
//! to SSE and notifications.

use serde::Serialize;
use sqlx::SqlitePool;

use crate::state::App;

pub const EVENTS: &[&str] = &[
    "bookAdded",
    "grab",
    "download",
    "downloadFailed",
    "licenceDenied",
    "licenceThrottled",
    "syncCompleted",
    "syncFailed",
    "accountAdded",
    "accountRemoved",
    "authExpired",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItem {
    pub id: i64,
    pub time: String,
    pub event: String,
    pub account_id: Option<String>,
    pub book_id: Option<String>,
    pub title: Option<String>,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    time: String,
    event: String,
    account_id: Option<String>,
    book_id: Option<String>,
    title: Option<String>,
    message: String,
    data: Option<String>,
}

impl From<Row> for HistoryItem {
    fn from(r: Row) -> Self {
        Self {
            id: r.id,
            time: r.time,
            event: r.event,
            account_id: r.account_id,
            book_id: r.book_id,
            title: r.title,
            message: r.message,
            data: r.data.and_then(|d| serde_json::from_str(&d).ok()),
        }
    }
}

/// A new history entry.
#[derive(Debug, Clone, Default)]
pub struct NewEvent {
    pub event: &'static str,
    pub account_id: Option<String>,
    pub book_id: Option<String>,
    pub title: Option<String>,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

impl NewEvent {
    pub fn new(event: &'static str, message: impl Into<String>) -> Self {
        Self { event, message: message.into(), ..Default::default() }
    }
    pub fn account(mut self, id: &str) -> Self {
        self.account_id = Some(id.to_string());
        self
    }
    pub fn book(mut self, id: &str, title: &str) -> Self {
        self.book_id = Some(id.to_string());
        self.title = Some(title.to_string());
        self
    }
    pub fn data(mut self, data: serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }
}

pub async fn insert(pool: &SqlitePool, e: &NewEvent) -> sqlx::Result<HistoryItem> {
    let time = crate::time::now_str();
    let data = e.data.as_ref().map(|d| d.to_string());
    let id = sqlx::query(
        "INSERT INTO history (time, event, account_id, book_id, title, message, data) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&time)
    .bind(e.event)
    .bind(&e.account_id)
    .bind(&e.book_id)
    .bind(&e.title)
    .bind(&e.message)
    .bind(&data)
    .execute(pool)
    .await?
    .last_insert_rowid();
    Ok(HistoryItem {
        id,
        time,
        event: e.event.to_string(),
        account_id: e.account_id.clone(),
        book_id: e.book_id.clone(),
        title: e.title.clone(),
        message: e.message.clone(),
        data: e.data.clone(),
    })
}

/// Record an event, publish it over SSE and hand it to the notifiers. Failures are
/// logged, never propagated: history must not break the action it describes.
pub async fn record(app: &App, e: NewEvent) {
    match insert(&app.db, &e).await {
        Ok(item) => {
            app.hub.publish("history", &item);
            app.notifier.dispatch(app, &item).await;
        }
        Err(err) => tracing::error!("Could not record {} event: {err}", e.event),
    }
}

/// Paged history, newest first.
pub async fn list(
    pool: &SqlitePool,
    events: &[String],
    account: Option<&str>,
    book: Option<&str>,
    page: i64,
    page_size: i64,
) -> sqlx::Result<(i64, Vec<HistoryItem>)> {
    let mut filter = String::from(" WHERE 1=1");
    if !events.is_empty() {
        filter.push_str(" AND event IN (SELECT value FROM json_each(?))");
    }
    if account.is_some() {
        filter.push_str(" AND account_id = ?");
    }
    if book.is_some() {
        filter.push_str(" AND book_id = ?");
    }
    let events_json = serde_json::to_string(events).unwrap_or_else(|_| "[]".into());

    let count_sql = format!("SELECT COUNT(*) FROM history{filter}");
    let mut q = sqlx::query_scalar::<_, i64>(&count_sql);
    if !events.is_empty() {
        q = q.bind(&events_json);
    }
    if let Some(a) = account {
        q = q.bind(a);
    }
    if let Some(b) = book {
        q = q.bind(b);
    }
    let total = q.fetch_one(pool).await?;

    let sql = format!(
        "SELECT id, time, event, account_id, book_id, title, message, data FROM history{filter} ORDER BY time DESC, id DESC LIMIT ? OFFSET ?"
    );
    let mut q = sqlx::query_as::<_, Row>(&sql);
    if !events.is_empty() {
        q = q.bind(&events_json);
    }
    if let Some(a) = account {
        q = q.bind(a);
    }
    if let Some(b) = book {
        q = q.bind(b);
    }
    let rows = q.bind(page_size).bind((page - 1) * page_size).fetch_all(pool).await?;
    Ok((total, rows.into_iter().map(HistoryItem::from).collect()))
}
