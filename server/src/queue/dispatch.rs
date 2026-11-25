//! The dispatcher: one task that decides which queued items may start, keeps every
//! waiting item's `waitingReason` current, and starts workers up to
//! `downloads.concurrency`.
//!
//! A single dispatcher (rather than workers pulling) means pacing decisions are made
//! in one place, in priority order, so two items of one Audible account can never both
//! be granted a licence slot.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{Local, TimeZone};

use crate::accounts;
use crate::pacing::{self, AccountPacing, Gate, PacingConfig};
use crate::queue::{self, Priority, QueueRow, QueueState};
use crate::state::AppState;
use crate::time;

const TICK: StdDuration = StdDuration::from_secs(30);

/// What the dispatcher decided for one item.
#[derive(Debug, PartialEq)]
enum Decision {
    Start,
    /// Eligible, but every worker is busy.
    Ready,
    Wait(String),
    /// The item can never run (its account is gone).
    Drop(String),
}

pub async fn run(app: AppState) {
    loop {
        if let Err(e) = dispatch_once(&app).await {
            tracing::error!("Queue dispatch failed: {e:#}");
        }
        tokio::select! {
            _ = app.shutdown.cancelled() => break,
            _ = app.queue.wake.notified() => {}
            _ = tokio::time::sleep(TICK) => {}
        }
    }
}

#[derive(sqlx::FromRow)]
struct AccountRow {
    account_id: String,
    auth_status: String,
}

pub async fn dispatch_once(app: &AppState) -> anyhow::Result<()> {
    if app.shutdown.is_cancelled() {
        return Ok(());
    }
    let settings = app.settings();
    let now = time::now();
    let pending = queue::list_pending(&app.db).await?;
    if pending.is_empty() {
        return Ok(());
    }

    let accounts: HashMap<String, String> = sqlx::query_as::<_, AccountRow>(
        "SELECT a.account_id, COALESCE(s.auth_status, 'unknown') AS auth_status
         FROM core.Accounts a LEFT JOIN account_state s ON s.account_id = a.account_id",
    )
    .fetch_all(&app.db)
    .await?
    .into_iter()
    .map(|r| (r.account_id, r.auth_status))
    .collect();

    let cfg = PacingConfig {
        daily_cap: settings.audible.daily_licence_cap,
        spacing_min_minutes: settings.audible.spacing_min_minutes,
        spacing_max_minutes: settings.audible.spacing_max_minutes,
    };
    let window = settings.backfill_window();
    let local_now = Local::now().naive_local();
    let mut free = (settings.downloads.concurrency as usize).saturating_sub(app.queue.running_count());
    let mut pacing_cache: HashMap<String, AccountPacing> = HashMap::new();

    for row in pending {
        let decision = decide(app, &row, &accounts, &cfg, window, local_now, now, free, &mut pacing_cache).await?;
        match decision {
            Decision::Start => {
                if start(app, &row).await? {
                    free -= 1;
                    if row.provider == "audible" {
                        if let Some(p) = pacing_cache.get_mut(&row.account_id) {
                            p.in_flight = true;
                        }
                    }
                }
            }
            Decision::Ready => set_waiting(app, &row, QueueState::Queued, None).await?,
            Decision::Wait(reason) => set_waiting(app, &row, QueueState::Waiting, Some(reason)).await?,
            Decision::Drop(reason) => {
                sqlx::query("UPDATE queue SET state = 'failed', error = ?, finished_at = ? WHERE id = ?")
                    .bind(&reason)
                    .bind(time::now_str())
                    .bind(row.id)
                    .execute(&app.db)
                    .await?;
                queue::publish_id(app, row.id).await;
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn decide(
    app: &AppState,
    row: &QueueRow,
    accounts: &HashMap<String, String>,
    cfg: &PacingConfig,
    window: Option<(chrono::NaiveTime, chrono::NaiveTime)>,
    local_now: chrono::NaiveDateTime,
    now: chrono::DateTime<chrono::Utc>,
    free: usize,
    pacing_cache: &mut HashMap<String, AccountPacing>,
) -> anyhow::Result<Decision> {
    let Some(auth) = accounts.get(&row.account_id) else {
        return Ok(Decision::Drop("The account that owns this book has been removed".into()));
    };
    if auth == "expired" {
        return Ok(Decision::Wait("Account sign-in expired; sign in again to continue".into()));
    }
    if let Some(nb) = row.not_before().filter(|nb| *nb > now) {
        let reason = row
            .waiting_reason
            .clone()
            .unwrap_or_else(|| format!("Retrying at {}", time::local_label(nb)));
        return Ok(Decision::Wait(reason));
    }
    if row.priority() == Priority::Backfill {
        if let Some((start, end)) = window {
            if !pacing::in_window(start, end, local_now.time()) {
                let open = pacing::next_window_open(start, local_now);
                let label = Local
                    .from_local_datetime(&open)
                    .earliest()
                    .map(|dt| time::local_label(dt.with_timezone(&chrono::Utc)))
                    .unwrap_or_else(|| start.format("%H:%M").to_string());
                return Ok(Decision::Wait(format!("Back-fill window opens at {label}")));
            }
        }
    }
    if row.provider == "audible" {
        if !pacing_cache.contains_key(&row.account_id) {
            let mut p = accounts::pacing(&app.db, &row.account_id, now).await?;
            p.in_flight = app.queue.licence_in_flight(&row.account_id);
            pacing_cache.insert(row.account_id.clone(), p);
        }
        let acct = &pacing_cache[&row.account_id];
        if let Gate::Wait { reason, .. } = pacing::licence_gate(cfg, acct, row.priority(), row.force, now) {
            return Ok(Decision::Wait(reason.describe()));
        }
    }
    Ok(if free > 0 { Decision::Start } else { Decision::Ready })
}

async fn set_waiting(app: &AppState, row: &QueueRow, state: QueueState, reason: Option<String>) -> anyhow::Result<()> {
    if row.state() == state && row.waiting_reason == reason {
        return Ok(());
    }
    sqlx::query("UPDATE queue SET state = ?, waiting_reason = ? WHERE id = ? AND state IN ('queued','waiting')")
        .bind(state.as_str())
        .bind(&reason)
        .bind(row.id)
        .execute(&app.db)
        .await?;
    queue::publish_id(app, row.id).await;
    Ok(())
}

/// Claim the row and spawn its worker. Returns false if another path got there first.
async fn start(app: &AppState, row: &QueueRow) -> anyhow::Result<bool> {
    let claimed = sqlx::query(
        "UPDATE queue SET state = 'planning', started_at = ?, waiting_reason = NULL, error = NULL
         WHERE id = ? AND state IN ('queued','waiting')",
    )
    .bind(time::now_str())
    .bind(row.id)
    .execute(&app.db)
    .await?
    .rows_affected()
        == 1;
    if !claimed {
        return Ok(false);
    }
    let cancel = app.shutdown.child_token();
    let user_cancelled = Arc::new(AtomicBool::new(false));
    app.queue.register(row.id, cancel.clone(), user_cancelled.clone());
    if row.provider == "audible" {
        app.queue.set_licence_in_flight(&row.account_id, true);
    }
    queue::publish_id(app, row.id).await;

    let app = app.clone();
    let id = row.id;
    tokio::spawn(async move {
        super::worker::run(app, id, cancel, user_cancelled).await;
    });
    Ok(true)
}

/// Items a previous process left mid-flight go back to the queue; their partial
/// downloads in `work/<id>` are reused.
pub async fn recover(pool: &sqlx::SqlitePool) -> sqlx::Result<u64> {
    let n = sqlx::query("UPDATE queue SET state = 'queued', waiting_reason = NULL WHERE state IN ('planning','running')")
        .execute(pool)
        .await?
        .rows_affected();
    sqlx::query("UPDATE licence_requests SET outcome = 'unknown' WHERE outcome = 'pending'")
        .execute(pool)
        .await?;
    Ok(n)
}
