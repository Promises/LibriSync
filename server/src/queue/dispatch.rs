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
    provider: String,
    auth_status: String,
    pacing_key: Option<String>,
}

/// What the dispatcher needs to know about an item's account.
struct AccountInfo {
    auth_status: String,
    pacing_key: String,
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

    // A local problem (disk, ffmpeg) pauses everything: each start would only spend a
    // licence on a book that can't be saved.
    if let Some(hold) = app.queue.hold() {
        let reason = hold.waiting_reason();
        for row in &pending {
            set_waiting(app, row, QueueState::Waiting, Some(reason.clone())).await?;
        }
        return Ok(());
    }

    let accounts: HashMap<String, AccountInfo> = sqlx::query_as::<_, AccountRow>(
        "SELECT a.account_id, COALESCE(a.provider, 'audible') AS provider,
                COALESCE(s.auth_status, 'unknown') AS auth_status, s.pacing_key
         FROM core.Accounts a LEFT JOIN account_state s ON s.account_id = a.account_id",
    )
    .fetch_all(&app.db)
    .await?
    .into_iter()
    .map(|r| {
        let pacing_key = r
            .pacing_key
            .unwrap_or_else(|| accounts::pacing_key_for(&r.provider, &r.account_id, None));
        (r.account_id, AccountInfo { auth_status: r.auth_status, pacing_key })
    })
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
                let licence_key = (row.provider == "audible")
                    .then(|| accounts.get(&row.account_id).map(|a| a.pacing_key.clone()))
                    .flatten();
                if start(app, &row, licence_key.clone()).await? {
                    free -= 1;
                    if let Some(p) = licence_key.and_then(|k| pacing_cache.get_mut(&k)) {
                        p.in_flight = true;
                    }
                }
            }
            Decision::Ready => set_waiting(app, &row, QueueState::Queued, None).await?,
            Decision::Wait(reason) => set_waiting(app, &row, QueueState::Waiting, Some(reason)).await?,
            Decision::Drop(reason) => {
                sqlx::query(
                    "UPDATE queue SET state = 'failed', error = ?, finished_at = ? WHERE id = ? AND state IN ('queued','waiting')",
                )
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
    accounts: &HashMap<String, AccountInfo>,
    cfg: &PacingConfig,
    window: Option<(chrono::NaiveTime, chrono::NaiveTime)>,
    local_now: chrono::NaiveDateTime,
    now: chrono::DateTime<chrono::Utc>,
    free: usize,
    pacing_cache: &mut HashMap<String, AccountPacing>,
) -> anyhow::Result<Decision> {
    let Some(account) = accounts.get(&row.account_id) else {
        return Ok(Decision::Drop("The account that owns this book has been removed".into()));
    };
    if account.auth_status == "expired" {
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
        // Pacing is per Amazon customer, shared by its marketplace accounts.
        let key = &account.pacing_key;
        if !pacing_cache.contains_key(key) {
            // In-flight first, then the database: a worker records a request's outcome
            // before clearing its mark, so if the mark is already gone, the read below
            // sees the outcome (the other order could miss both).
            let in_flight = app.queue.licence_in_flight(key);
            let mut p = accounts::pacing(&app.db, key, now).await?;
            p.in_flight = in_flight;
            pacing_cache.insert(key.clone(), p);
        }
        let acct = &pacing_cache[key];
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
///
/// The item is registered *before* it is claimed, so any row in `planning`/`running`
/// has a registration that a cancel can reach.
async fn start(app: &AppState, row: &QueueRow, licence_key: Option<String>) -> anyhow::Result<bool> {
    let cancel = app.shutdown.child_token();
    let user_cancelled = Arc::new(AtomicBool::new(false));
    let generation = app.queue.register(row.id, cancel.clone(), user_cancelled.clone());
    let claimed = sqlx::query(
        "UPDATE queue SET state = 'planning', started_at = ?, waiting_reason = NULL, error = NULL
         WHERE id = ? AND state IN ('queued','waiting')",
    )
    .bind(time::now_str())
    .bind(row.id)
    .execute(&app.db)
    .await;
    let claimed = match claimed {
        Ok(r) => r.rows_affected() == 1,
        Err(e) => {
            app.queue.unregister(row.id, generation);
            return Err(e.into());
        }
    };
    if !claimed {
        app.queue.unregister(row.id, generation);
        return Ok(false);
    }
    if let Some(key) = &licence_key {
        app.queue.set_licence_in_flight(key, row.id);
    }
    queue::publish_id(app, row.id).await;

    let app = app.clone();
    let id = row.id;
    tokio::spawn(async move {
        super::worker::run(app, id, generation, licence_key, cancel, user_cancelled).await;
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
