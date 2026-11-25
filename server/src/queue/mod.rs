//! The download queue: persisted rows in `server.db`, plus in-memory runtime state
//! (running items, their progress, Audible licence requests in flight).
//!
//! A book has at most one *slot* row (state queued/waiting/planning/running/failed,
//! enforced by a partial unique index); completed and cancelled rows are kept as
//! history until housekeeping prunes them.

pub mod dispatch;
pub mod worker;

#[cfg(test)]
mod flow_tests;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use chrono::{DateTime, Utc};
use rust_core::pipeline::{Progress, Stage};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::state::App;
use crate::time;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Manual = 0,
    New = 1,
    Backfill = 2,
}

impl Priority {
    pub fn from_i64(v: i64) -> Self {
        match v {
            0 => Priority::Manual,
            1 => Priority::New,
            _ => Priority::Backfill,
        }
    }
    pub fn as_i64(self) -> i64 {
        self as i64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueueState {
    Queued,
    Waiting,
    Planning,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl QueueState {
    pub fn as_str(self) -> &'static str {
        match self {
            QueueState::Queued => "queued",
            QueueState::Waiting => "waiting",
            QueueState::Planning => "planning",
            QueueState::Running => "running",
            QueueState::Completed => "completed",
            QueueState::Failed => "failed",
            QueueState::Cancelled => "cancelled",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "waiting" => QueueState::Waiting,
            "planning" => QueueState::Planning,
            "running" => QueueState::Running,
            "completed" => QueueState::Completed,
            "failed" => QueueState::Failed,
            "cancelled" => QueueState::Cancelled,
            _ => QueueState::Queued,
        }
    }
}

/// States that occupy a book's single queue slot.
pub const SLOT_STATES_SQL: &str = "('queued','waiting','planning','running','failed')";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct QueueRow {
    pub id: i64,
    pub book_id: String,
    pub account_id: String,
    pub provider: String,
    pub title: String,
    pub authors: String,
    pub priority: i64,
    pub state: String,
    pub force: bool,
    pub waiting_reason: Option<String>,
    pub not_before: Option<String>,
    pub attempts: i64,
    pub error: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

impl QueueRow {
    pub fn priority(&self) -> Priority {
        Priority::from_i64(self.priority)
    }
    pub fn state(&self) -> QueueState {
        QueueState::parse(&self.state)
    }
    pub fn not_before(&self) -> Option<DateTime<Utc>> {
        self.not_before.as_deref().and_then(time::parse)
    }
}

/// `QueueItem` in API.md.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    pub id: String,
    pub book_id: String,
    pub account_id: String,
    pub provider: String,
    pub title: String,
    pub authors: Vec<String>,
    pub priority: Priority,
    pub state: QueueState,
    pub waiting_reason: Option<String>,
    pub not_before: Option<String>,
    pub stage: Option<Stage>,
    pub part: Option<usize>,
    pub parts: Option<usize>,
    pub bytes_done: Option<u64>,
    pub bytes_total: Option<u64>,
    pub bytes_per_sec: Option<u64>,
    pub progress: Option<f64>,
    pub attempts: i64,
    pub error: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

/// Overall completion estimate across stages: downloading is most of the wall time;
/// the post-processing stages share the rest.
pub fn overall_progress(p: &Progress) -> f64 {
    let frac = p.fraction.map(|f| f.clamp(0.0, 1.0) as f64);
    let (lo, hi) = match p.stage {
        Stage::Downloading => {
            let parts = p.parts.max(1) as f64;
            let part_frac = frac.or_else(|| {
                p.bytes_total
                    .filter(|t| *t > 0)
                    .map(|t| (p.bytes_done as f64 / t as f64).min(1.0))
            });
            let done = (p.part.max(1) - 1) as f64 + part_frac.unwrap_or(0.0);
            return 0.7 * (done / parts).min(1.0);
        }
        Stage::Decrypting | Stage::Extracting | Stage::Joining => (0.7, 0.85),
        Stage::Validating => (0.85, 0.9),
        Stage::Encoding => (0.9, 0.98),
        Stage::Finalizing => (0.98, 1.0),
    };
    lo + (hi - lo) * frac.unwrap_or(0.0)
}

struct Running {
    cancel: CancellationToken,
    user_cancelled: Arc<AtomicBool>,
    progress: Option<Progress>,
    last_emit: Option<Instant>,
}

/// In-memory queue state shared by the dispatcher, workers and API.
#[derive(Default)]
pub struct Runtime {
    pub wake: Notify,
    running: Mutex<HashMap<i64, Running>>,
    /// Audible accounts with a licence request in flight.
    in_flight: Mutex<HashSet<String>>,
}

impl Runtime {
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    pub fn running_count(&self) -> usize {
        self.running.lock().map(|r| r.len()).unwrap_or(0)
    }

    pub fn register(&self, id: i64, cancel: CancellationToken, user_cancelled: Arc<AtomicBool>) {
        if let Ok(mut r) = self.running.lock() {
            r.insert(id, Running { cancel, user_cancelled, progress: None, last_emit: None });
        }
    }

    pub fn unregister(&self, id: i64) {
        if let Ok(mut r) = self.running.lock() {
            r.remove(&id);
        }
    }

    /// Cancel a running item on the user's behalf. Returns false if it isn't running.
    pub fn cancel(&self, id: i64) -> bool {
        let Ok(r) = self.running.lock() else { return false };
        match r.get(&id) {
            Some(item) => {
                item.user_cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
                item.cancel.cancel();
                true
            }
            None => false,
        }
    }

    pub fn is_running(&self, id: i64) -> bool {
        self.running.lock().map(|r| r.contains_key(&id)).unwrap_or(false)
    }

    /// Store progress; returns true when it should be published (stage changed, or at
    /// least a second since the last publish).
    pub fn set_progress(&self, id: i64, p: Progress) -> bool {
        let Ok(mut r) = self.running.lock() else { return false };
        let Some(item) = r.get_mut(&id) else { return false };
        let stage_changed = item.progress.as_ref().map(|old| old.stage != p.stage || old.part != p.part).unwrap_or(true);
        let due = item.last_emit.map(|t| t.elapsed().as_millis() >= 1000).unwrap_or(true);
        item.progress = Some(p);
        if stage_changed || due {
            item.last_emit = Some(Instant::now());
            true
        } else {
            false
        }
    }

    pub fn progress(&self, id: i64) -> Option<Progress> {
        self.running.lock().ok().and_then(|r| r.get(&id).and_then(|i| i.progress.clone()))
    }

    pub fn licence_in_flight(&self, account: &str) -> bool {
        self.in_flight.lock().map(|s| s.contains(account)).unwrap_or(false)
    }

    pub fn set_licence_in_flight(&self, account: &str, on: bool) {
        if let Ok(mut s) = self.in_flight.lock() {
            if on {
                s.insert(account.to_string());
            } else {
                s.remove(account);
            }
        }
    }

    /// Cancel every running item without marking it user-cancelled (shutdown).
    pub fn cancel_all(&self) {
        if let Ok(r) = self.running.lock() {
            for item in r.values() {
                item.cancel.cancel();
            }
        }
    }
}

pub fn to_item(row: &QueueRow, rt: &Runtime) -> QueueItem {
    let progress = rt.progress(row.id);
    let state = row.state();
    let active = matches!(state, QueueState::Running);
    let p = progress.as_ref().filter(|_| active);
    QueueItem {
        id: row.id.to_string(),
        book_id: row.book_id.clone(),
        account_id: row.account_id.clone(),
        provider: row.provider.clone(),
        title: row.title.clone(),
        authors: serde_json::from_str(&row.authors).unwrap_or_default(),
        priority: row.priority(),
        state,
        waiting_reason: row.waiting_reason.clone(),
        not_before: row.not_before.clone(),
        stage: p.map(|p| p.stage),
        part: p.map(|p| p.part),
        parts: p.map(|p| p.parts),
        bytes_done: p.map(|p| p.bytes_done),
        bytes_total: p.and_then(|p| p.bytes_total),
        bytes_per_sec: p.and_then(|p| p.bytes_per_sec),
        progress: match state {
            QueueState::Completed => Some(1.0),
            _ => p.map(overall_progress),
        },
        attempts: row.attempts,
        error: row.error.clone(),
        created_at: row.created_at.clone(),
        started_at: row.started_at.clone(),
        finished_at: row.finished_at.clone(),
    }
}

const COLUMNS: &str = "id, book_id, account_id, provider, title, authors, priority, state, force, waiting_reason, not_before, attempts, error, created_at, started_at, finished_at";

pub async fn get(pool: &SqlitePool, id: i64) -> sqlx::Result<Option<QueueRow>> {
    sqlx::query_as::<_, QueueRow>(&format!("SELECT {COLUMNS} FROM queue WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn slot_for_book(pool: &SqlitePool, book_id: &str) -> sqlx::Result<Option<QueueRow>> {
    sqlx::query_as::<_, QueueRow>(&format!(
        "SELECT {COLUMNS} FROM queue WHERE book_id = ? AND state IN {SLOT_STATES_SQL}"
    ))
    .bind(book_id)
    .fetch_optional(pool)
    .await
}

/// Active items (not completed/cancelled), in the order the worker would take them:
/// what is running first, then by priority, then oldest first.
pub async fn list_active(pool: &SqlitePool) -> sqlx::Result<Vec<QueueRow>> {
    sqlx::query_as::<_, QueueRow>(&format!(
        "SELECT {COLUMNS} FROM queue WHERE state IN {SLOT_STATES_SQL}
         ORDER BY CASE state WHEN 'running' THEN 0 WHEN 'planning' THEN 0 WHEN 'failed' THEN 2 ELSE 1 END,
                  priority, created_at, id"
    ))
    .fetch_all(pool)
    .await
}

/// Items the dispatcher may start, best first.
pub async fn list_pending(pool: &SqlitePool) -> sqlx::Result<Vec<QueueRow>> {
    sqlx::query_as::<_, QueueRow>(&format!(
        "SELECT {COLUMNS} FROM queue WHERE state IN ('queued','waiting') ORDER BY priority, created_at, id"
    ))
    .fetch_all(pool)
    .await
}

/// Publish an item's current state over SSE.
pub fn publish(app: &App, row: &QueueRow) {
    let item = to_item(row, &app.queue);
    app.hub.publish("queue", &item);
    if matches!(item.state, QueueState::Completed | QueueState::Cancelled) {
        app.hub.publish("queueRemoved", &serde_json::json!({ "id": item.id }));
    }
}

/// Re-read a row and publish it.
pub async fn publish_id(app: &App, id: i64) {
    match get(&app.db, id).await {
        Ok(Some(row)) => publish(app, &row),
        Ok(None) => app.hub.publish("queueRemoved", &serde_json::json!({ "id": id.to_string() })),
        Err(e) => tracing::warn!("Could not read queue item {id}: {e}"),
    }
}

#[derive(Debug)]
pub enum EnqueueError {
    NotFound,
    /// No signed-in account owns this book.
    NoAccount,
    AlreadyDownloaded,
    Skipped,
    /// Title backoff after a licence denial (only manual downloads bypass it).
    Backoff(DateTime<Utc>),
    /// The owning Audible account is cooling down and the request wasn't forced.
    Cooldown(DateTime<Utc>),
    Db(sqlx::Error),
}

impl From<sqlx::Error> for EnqueueError {
    fn from(e: sqlx::Error) -> Self {
        EnqueueError::Db(e)
    }
}

impl EnqueueError {
    /// Machine-readable reason for bulk results.
    pub fn reason(&self) -> String {
        match self {
            EnqueueError::NotFound => "not_found".into(),
            EnqueueError::NoAccount => "no_account".into(),
            EnqueueError::AlreadyDownloaded => "downloaded".into(),
            EnqueueError::Skipped => "skipped".into(),
            EnqueueError::Backoff(_) => "backoff".into(),
            EnqueueError::Cooldown(_) => "account_cooldown".into(),
            EnqueueError::Db(_) => "internal".into(),
        }
    }
}

#[derive(sqlx::FromRow)]
struct OwnerRow {
    account_id: String,
    cooldown_until: Option<String>,
    auth_status: String,
    enabled: bool,
}

/// Queue `book_id`. Manual requests also cover downloaded, skipped and backed-off
/// books; an existing queued item is upgraded to the higher priority instead of
/// duplicated, and a failed one is reset.
pub async fn enqueue(app: &App, book_id: &str, priority: Priority, force: bool) -> Result<QueueRow, EnqueueError> {
    let pool = &app.db;
    let now = time::now();
    let book: Option<(String, String)> = sqlx::query_as(
        "SELECT title, COALESCE(source, 'audible') FROM core.Books WHERE audible_product_id = ?",
    )
    .bind(book_id)
    .fetch_optional(pool)
    .await?;
    let Some((title, provider)) = book else { return Err(EnqueueError::NotFound) };

    if priority != Priority::Manual {
        let downloaded: Option<i64> = sqlx::query_scalar("SELECT 1 FROM downloads WHERE book_id = ?")
            .bind(book_id)
            .fetch_optional(pool)
            .await?;
        if downloaded.is_some() {
            return Err(EnqueueError::AlreadyDownloaded);
        }
        let skipped: Option<i64> = sqlx::query_scalar("SELECT 1 FROM skips WHERE book_id = ?")
            .bind(book_id)
            .fetch_optional(pool)
            .await?;
        if skipped.is_some() {
            return Err(EnqueueError::Skipped);
        }
        let backoff: Option<String> =
            sqlx::query_scalar("SELECT until FROM title_backoff WHERE book_id = ? AND until > ?")
                .bind(book_id)
                .bind(time::fmt(now))
                .fetch_optional(pool)
                .await?;
        if let Some(until) = backoff.as_deref().and_then(time::parse) {
            return Err(EnqueueError::Backoff(until));
        }
    }

    // Pick the owning account: signed in, enabled and not cooling down, if possible.
    let owners: Vec<OwnerRow> = sqlx::query_as(
        "SELECT a.account_id, s.cooldown_until, COALESCE(s.auth_status, 'unknown') AS auth_status,
                COALESCE(s.enabled, 1) AS enabled
         FROM core.BookAccounts ba
         JOIN core.Books b ON b.book_id = ba.book_id
         JOIN core.Accounts a ON a.account_id = ba.account
         LEFT JOIN account_state s ON s.account_id = a.account_id
         WHERE b.audible_product_id = ? AND ba.is_deleted = 0
           AND COALESCE(a.provider, 'audible') = COALESCE(b.source, 'audible')
         ORDER BY a.account_id",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await?;
    let cooling = |o: &OwnerRow| {
        o.cooldown_until
            .as_deref()
            .and_then(time::parse)
            .filter(|u| *u > now)
    };
    let Some(owner) = owners
        .iter()
        .min_by_key(|o| (o.auth_status == "expired", cooling(o).is_some(), !o.enabled))
    else {
        return Err(EnqueueError::NoAccount);
    };
    if priority == Priority::Manual && !force {
        if let Some(until) = cooling(owner) {
            return Err(EnqueueError::Cooldown(until));
        }
    }

    let authors: Vec<String> = sqlx::query_scalar(
        "SELECT c.name FROM core.BookContributors bc
         JOIN core.Contributors c ON c.contributor_id = bc.contributor_id
         JOIN core.Books b ON b.book_id = bc.book_id
         WHERE b.audible_product_id = ? AND bc.role = 1 ORDER BY bc.\"order\"",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await?;
    let authors_json = serde_json::to_string(&authors).unwrap_or_else(|_| "[]".into());

    if let Some(existing) = slot_for_book(pool, book_id).await? {
        return upgrade_slot(app, existing, &owner.account_id, priority, force).await;
    }

    let inserted = sqlx::query(
        "INSERT INTO queue (book_id, account_id, provider, title, authors, priority, state, force, created_at)
         VALUES (?, ?, ?, ?, ?, ?, 'queued', ?, ?)",
    )
    .bind(book_id)
    .bind(&owner.account_id)
    .bind(&provider)
    .bind(&title)
    .bind(&authors_json)
    .bind(priority.as_i64())
    .bind(force)
    .bind(time::fmt(now))
    .execute(pool)
    .await;
    let id = match inserted {
        Ok(r) => r.last_insert_rowid(),
        // Lost a race with another enqueue of the same book: use its row.
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            if let Some(existing) = slot_for_book(pool, book_id).await? {
                return upgrade_slot(app, existing, &owner.account_id, priority, force).await;
            }
            return Err(EnqueueError::Db(sqlx::Error::Database(e)));
        }
        Err(e) => return Err(e.into()),
    };
    let row = get(pool, id).await?.ok_or(EnqueueError::NotFound)?;
    publish(app, &row);
    app.queue.wake();
    Ok(row)
}

async fn upgrade_slot(
    app: &App,
    existing: QueueRow,
    account_id: &str,
    priority: Priority,
    force: bool,
) -> Result<QueueRow, EnqueueError> {
    let pool = &app.db;
    match existing.state() {
        QueueState::Queued | QueueState::Waiting => {
            let p = existing.priority().min(priority);
            sqlx::query("UPDATE queue SET priority = ?, force = (force OR ?) WHERE id = ?")
                .bind(p.as_i64())
                .bind(force)
                .bind(existing.id)
                .execute(pool)
                .await?;
        }
        QueueState::Failed => {
            sqlx::query(
                "UPDATE queue SET state = 'queued', priority = ?, force = ?, account_id = ?, attempts = 0,
                        error = NULL, not_before = NULL, waiting_reason = NULL, started_at = NULL, finished_at = NULL
                 WHERE id = ?",
            )
            .bind(priority.as_i64())
            .bind(force)
            .bind(account_id)
            .bind(existing.id)
            .execute(pool)
            .await?;
        }
        _ => return Ok(existing),
    }
    let row = get(pool, existing.id).await?.ok_or(EnqueueError::NotFound)?;
    publish(app, &row);
    app.queue.wake();
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(stage: Stage, part: usize, parts: usize, fraction: Option<f32>) -> Progress {
        Progress {
            stage,
            part,
            parts,
            bytes_done: 0,
            bytes_total: None,
            bytes_per_sec: None,
            fraction,
        }
    }

    #[test]
    fn overall_progress_is_monotonic_across_stages() {
        let seq = [
            progress(Stage::Downloading, 1, 2, Some(0.0)),
            progress(Stage::Downloading, 1, 2, Some(0.5)),
            progress(Stage::Downloading, 2, 2, Some(0.0)),
            progress(Stage::Downloading, 2, 2, Some(1.0)),
            progress(Stage::Joining, 1, 1, Some(0.5)),
            progress(Stage::Validating, 1, 1, None),
            progress(Stage::Encoding, 1, 1, Some(0.5)),
            progress(Stage::Finalizing, 1, 1, Some(1.0)),
        ];
        let values: Vec<f64> = seq.iter().map(overall_progress).collect();
        assert!(values.windows(2).all(|w| w[0] <= w[1]), "{values:?}");
        assert_eq!(values[0], 0.0);
        assert!((values[3] - 0.7).abs() < 1e-9);
        assert!((values[7] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn download_progress_falls_back_to_bytes() {
        let mut p = progress(Stage::Downloading, 1, 1, None);
        p.bytes_done = 50;
        p.bytes_total = Some(100);
        assert!((overall_progress(&p) - 0.35).abs() < 1e-9);
    }

    #[test]
    fn priorities_order_manual_new_backfill() {
        let mut v = vec![Priority::Backfill, Priority::Manual, Priority::New];
        v.sort();
        assert_eq!(v, vec![Priority::Manual, Priority::New, Priority::Backfill]);
        for p in v {
            assert_eq!(Priority::from_i64(p.as_i64()), p);
        }
    }

    #[test]
    fn progress_publishing_is_throttled_but_stage_changes_go_through() {
        let rt = Runtime::default();
        rt.register(1, CancellationToken::new(), Arc::new(AtomicBool::new(false)));
        assert!(rt.set_progress(1, progress(Stage::Downloading, 1, 1, Some(0.1))));
        assert!(!rt.set_progress(1, progress(Stage::Downloading, 1, 1, Some(0.2))));
        assert!(rt.set_progress(1, progress(Stage::Validating, 1, 1, None)));
        assert!(!rt.set_progress(2, progress(Stage::Validating, 1, 1, None)), "unknown item");
    }
}
