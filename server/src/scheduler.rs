//! Scheduled tasks (System → Tasks): library sync, back-fill, client-profile refresh
//! and housekeeping. Each runs on its own timer and can be run on demand.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use tokio::sync::Notify;

use crate::accounts;
use crate::auth;
use crate::backfill;
use crate::queue::SLOT_STATES_SQL;
use crate::state::AppState;
use crate::sync;
use crate::time;

const HISTORY_DAYS: i64 = 90;
const FINISHED_QUEUE_DAYS: i64 = 30;
const LICENCE_LOG_DAYS: i64 = 30;
/// How often the sync task looks for accounts that are due.
const SYNC_CHECK: Duration = Duration::minutes(5);
/// Let the server settle (and the UI connect) before the first scheduled work.
const STARTUP_DELAY: StdDuration = StdDuration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskName {
    Sync,
    Backfill,
    ClientProfiles,
    Housekeeping,
}

impl TaskName {
    pub const ALL: [TaskName; 4] = [TaskName::Sync, TaskName::Backfill, TaskName::ClientProfiles, TaskName::Housekeeping];

    pub fn as_str(self) -> &'static str {
        match self {
            TaskName::Sync => "sync",
            TaskName::Backfill => "backfill",
            TaskName::ClientProfiles => "clientProfiles",
            TaskName::Housekeeping => "housekeeping",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == s)
    }

    fn display_name(self) -> &'static str {
        match self {
            TaskName::Sync => "Library sync",
            TaskName::Backfill => "Back-fill",
            TaskName::ClientProfiles => "Refresh client profiles",
            TaskName::Housekeeping => "Housekeeping",
        }
    }

    /// How often the task's timer fires.
    fn period(self) -> Duration {
        match self {
            TaskName::Sync => SYNC_CHECK,
            TaskName::Backfill => Duration::minutes(15),
            TaskName::ClientProfiles => Duration::hours(24),
            TaskName::Housekeeping => Duration::hours(24),
        }
    }
}

/// `ScheduledTask` in API.md.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledTask {
    pub name: &'static str,
    pub display_name: &'static str,
    pub interval_minutes: i64,
    pub last_run: Option<String>,
    pub next_run: Option<String>,
    pub running: bool,
}

#[derive(Default, Clone)]
struct TaskState {
    last_run: Option<DateTime<Utc>>,
    next_run: Option<DateTime<Utc>>,
    running: bool,
}

pub struct Tasks {
    state: Mutex<HashMap<TaskName, TaskState>>,
    run_now: HashMap<TaskName, Notify>,
}

impl Tasks {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(HashMap::new()),
            run_now: TaskName::ALL.into_iter().map(|t| (t, Notify::new())).collect(),
        }
    }

    fn get(&self, t: TaskName) -> TaskState {
        self.state.lock().ok().and_then(|m| m.get(&t).cloned()).unwrap_or_default()
    }

    fn update(&self, t: TaskName, f: impl FnOnce(&mut TaskState)) {
        if let Ok(mut m) = self.state.lock() {
            f(m.entry(t).or_default());
        }
    }

    /// Ask a task to run now. Returns false if it is already running.
    pub fn trigger(&self, t: TaskName) -> bool {
        if self.get(t).running {
            return false;
        }
        if let Some(n) = self.run_now.get(&t) {
            n.notify_one();
        }
        true
    }
}

pub async fn view(app: &AppState, t: TaskName) -> ScheduledTask {
    let st = app.tasks.get(t);
    let settings = app.settings();
    let (interval, next_run) = match t {
        // The sync task checks often, but each account syncs every intervalHours;
        // show that, and the earliest account due.
        TaskName::Sync => {
            let next = accounts::list_sched(&app.db)
                .await
                .unwrap_or_default()
                .into_iter()
                .filter(|a| a.enabled)
                .filter_map(|a| a.next_sync.as_deref().and_then(time::parse).or(st.next_run))
                .min()
                .or(st.next_run);
            (settings.sync.interval_hours as i64 * 60, next)
        }
        _ => (t.period().num_minutes(), st.next_run),
    };
    ScheduledTask {
        name: t.as_str(),
        display_name: t.display_name(),
        interval_minutes: interval,
        last_run: st.last_run.map(time::fmt),
        next_run: next_run.map(time::fmt),
        running: st.running,
    }
}

pub async fn list(app: &AppState) -> Vec<ScheduledTask> {
    let mut out = Vec::new();
    for t in TaskName::ALL {
        out.push(view(app, t).await);
    }
    out
}

async fn publish(app: &AppState, t: TaskName) {
    let v = view(app, t).await;
    app.hub.publish("task", &v);
}

/// Start every task's timer.
pub async fn start(app: &AppState) {
    let stored: Vec<(String, Option<String>)> = sqlx::query_as("SELECT name, last_run FROM task_runs")
        .fetch_all(&app.db)
        .await
        .unwrap_or_default();
    for (name, last) in stored {
        if let Some(t) = TaskName::parse(&name) {
            app.tasks.update(t, |s| s.last_run = last.as_deref().and_then(time::parse));
        }
    }
    for t in TaskName::ALL {
        tokio::spawn(task_loop(app.clone(), t));
    }
}

async fn task_loop(app: AppState, t: TaskName) {
    let now = time::now();
    // Daily tasks resume their schedule across restarts; the rest start shortly after boot.
    let first = match (t, app.tasks.get(t).last_run) {
        (TaskName::ClientProfiles | TaskName::Housekeeping, Some(last)) if last + t.period() > now => last + t.period(),
        _ => now + Duration::from_std(STARTUP_DELAY).unwrap_or_default(),
    };
    app.tasks.update(t, |s| s.next_run = Some(first));
    loop {
        let next = app.tasks.get(t).next_run.unwrap_or_else(time::now);
        let wait = (next - time::now()).to_std().unwrap_or_default();
        let Some(trigger) = app.tasks.run_now.get(&t) else { return };
        let manual = tokio::select! {
            _ = app.shutdown.cancelled() => return,
            _ = tokio::time::sleep(wait) => false,
            _ = trigger.notified() => true,
        };
        run(&app, t, manual).await;
        app.tasks.update(t, |s| s.next_run = Some(time::now() + t.period()));
        publish(&app, t).await;
    }
}

async fn run(app: &AppState, t: TaskName, manual: bool) {
    app.tasks.update(t, |s| s.running = true);
    publish(app, t).await;
    let started = time::now();
    let result = match t {
        TaskName::Sync => run_sync(app, manual).await,
        TaskName::Backfill => {
            backfill::run_once(app).await;
            Ok(())
        }
        TaskName::ClientProfiles => {
            let max_age = if manual { chrono::Duration::zero() } else { rust_core::client_profiles::DAILY };
            rust_core::client_profiles::refresh(&app.core, max_age)
                .await
                .map(|o| {
                    if o.changed {
                        tracing::info!("Client profiles updated (issued {})", o.profiles.issued);
                    }
                })
                .map_err(|e| anyhow::anyhow!("client profile refresh failed: {e}"))
        }
        TaskName::Housekeeping => housekeeping(app).await,
    };
    if let Err(e) = result {
        tracing::warn!("Task {} failed: {e:#}", t.as_str());
    }
    app.tasks.update(t, |s| {
        s.running = false;
        s.last_run = Some(started);
    });
    if let Err(e) = sqlx::query("INSERT INTO task_runs (name, last_run) VALUES (?, ?) ON CONFLICT(name) DO UPDATE SET last_run = excluded.last_run")
        .bind(t.as_str())
        .bind(time::fmt(started))
        .execute(&app.db)
        .await
    {
        tracing::warn!("Could not record task run: {e}");
    }
}

/// Sync every enabled account that is due (all enabled accounts when run by hand),
/// one after another so the stores never see a burst.
async fn run_sync(app: &AppState, manual: bool) -> anyhow::Result<()> {
    let now = time::now();
    for a in accounts::list_sched(&app.db).await? {
        if !a.enabled {
            continue;
        }
        let due = a.next_sync.as_deref().and_then(time::parse).is_none_or(|n| n <= now);
        if manual || (due && a.auth_status != "expired") {
            if let Err(e) = sync::sync_account(app, &a.account_id).await {
                tracing::error!("Scheduled sync failed: {e:#}");
            }
        }
    }
    Ok(())
}

async fn housekeeping(app: &AppState) -> anyhow::Result<()> {
    let now = time::now();
    let pool = &app.db;
    let history = sqlx::query("DELETE FROM history WHERE time < ?")
        .bind(time::fmt(now - Duration::days(HISTORY_DAYS)))
        .execute(pool)
        .await?
        .rows_affected();
    let queue = sqlx::query("DELETE FROM queue WHERE state IN ('completed','cancelled') AND finished_at < ?")
        .bind(time::fmt(now - Duration::days(FINISHED_QUEUE_DAYS)))
        .execute(pool)
        .await?
        .rows_affected();
    sqlx::query("DELETE FROM licence_requests WHERE time < ?")
        .bind(time::fmt(now - Duration::days(LICENCE_LOG_DAYS)))
        .execute(pool)
        .await?;
    auth::prune_sessions(pool).await?;
    let work = prune_work_dirs(app).await?;
    tracing::info!("Housekeeping: pruned {history} history row(s), {queue} finished queue row(s), {work} stale work dir(s)");

    // Re-check ffmpeg so an upgraded image or fixed path shows up without a restart.
    let status = app.executor.check_tools().await;
    app.set_tools_status(status);
    Ok(())
}

/// Remove `work/<id>` directories that no live, failed or recently cancelled queue
/// item can resume from.
async fn prune_work_dirs(app: &AppState) -> anyhow::Result<usize> {
    let dir = app.cfg.work_dir();
    let Ok(mut entries) = tokio::fs::read_dir(&dir).await else { return Ok(0) };
    let keep: Vec<i64> = sqlx::query_scalar(&format!(
        "SELECT id FROM queue WHERE state IN {SLOT_STATES_SQL} OR (state = 'cancelled' AND finished_at > ?)"
    ))
    .bind(time::fmt(time::now() - backfill::CANCELLED_GRACE))
    .fetch_all(&app.db)
    .await?;
    let mut removed = 0;
    while let Some(entry) = entries.next_entry().await? {
        let Some(id) = entry.file_name().to_str().and_then(|n| n.parse::<i64>().ok()) else { continue };
        if keep.contains(&id) || app.queue.is_running(id) {
            continue;
        }
        if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
            match tokio::fs::remove_dir_all(entry.path()).await {
                Ok(()) => removed += 1,
                Err(e) => tracing::warn!("Could not remove stale work dir {}: {e}", entry.path().display()),
            }
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_names_round_trip() {
        for t in TaskName::ALL {
            assert_eq!(TaskName::parse(t.as_str()), Some(t));
        }
        assert_eq!(TaskName::parse("nope"), None);
    }
}
