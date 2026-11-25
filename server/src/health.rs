//! Health checks (`GET /api/v1/health`) and the monitor that announces changes.

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration as StdDuration;

use serde::Serialize;

use crate::state::AppState;
use crate::time;

const MIN_FREE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MONITOR_INTERVAL: StdDuration = StdDuration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct HealthItem {
    pub source: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub message: String,
}

impl HealthItem {
    fn new(source: impl Into<String>, kind: &'static str, message: impl Into<String>) -> Self {
        Self { source: source.into(), kind, message: message.into() }
    }
}

/// Can we create files in `dir`?
pub fn writable(dir: &Path) -> Result<(), String> {
    if !dir.is_dir() {
        return Err(format!("{} does not exist or is not a directory", dir.display()));
    }
    let probe = dir.join(format!(".librisync-write-test-{}", uuid::Uuid::new_v4()));
    std::fs::write(&probe, b"").map_err(|e| format!("{} is not writable: {e}", dir.display()))?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

#[derive(sqlx::FromRow)]
struct AccountHealth {
    account_id: String,
    account_name: String,
    auth_status: String,
    cooldown_until: Option<String>,
    last_sync_ok: Option<bool>,
    last_sync_message: Option<String>,
}

/// Current problems. An empty list means healthy.
pub async fn check(app: &AppState) -> Vec<HealthItem> {
    let mut items = Vec::new();

    match app.tools_status() {
        Some(Ok(_)) => {}
        Some(Err(e)) => items.push(HealthItem::new("ffmpeg", "error", format!("ffmpeg is unusable: {e}"))),
        None => items.push(HealthItem::new("ffmpeg", "notice", "ffmpeg has not been checked yet")),
    }

    let lib = app.cfg.library_dir.clone();
    let lib_check = tokio::task::spawn_blocking(move || writable(&lib).map(|_| fs2::available_space(&lib)))
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
    match lib_check {
        Err(e) => items.push(HealthItem::new("libraryDir", "error", e)),
        Ok(Ok(free)) if free < MIN_FREE_BYTES => items.push(HealthItem::new(
            "disk",
            "warning",
            format!("Only {:.1} GB free in the library folder", free as f64 / 1e9),
        )),
        Ok(Ok(_)) => {}
        Ok(Err(e)) => items.push(HealthItem::new("disk", "notice", format!("Could not read free space: {e}"))),
    }

    let now = time::now();
    match sqlx::query_as::<_, AccountHealth>(
        "SELECT a.account_id, a.account_name, COALESCE(s.auth_status, 'unknown') AS auth_status,
                s.cooldown_until, s.last_sync_ok, s.last_sync_message
         FROM core.Accounts a LEFT JOIN account_state s ON s.account_id = a.account_id
         WHERE COALESCE(s.enabled, 1) = 1",
    )
    .fetch_all(&app.db)
    .await
    {
        Ok(rows) => {
            for a in rows {
                let source = format!("account:{}", a.account_id);
                if a.auth_status == "expired" {
                    items.push(HealthItem::new(
                        &source,
                        "error",
                        format!("{}: sign-in expired; sign in again", a.account_name),
                    ));
                    continue;
                }
                if let Some(until) = a.cooldown_until.as_deref().and_then(time::parse).filter(|u| *u > now) {
                    items.push(HealthItem::new(
                        &source,
                        "warning",
                        format!(
                            "{}: Audible throttled licence requests; downloads paused until {}",
                            a.account_name,
                            time::local_label(until)
                        ),
                    ));
                }
                if a.last_sync_ok == Some(false) {
                    items.push(HealthItem::new(
                        &source,
                        "warning",
                        format!(
                            "{}: last sync failed{}",
                            a.account_name,
                            a.last_sync_message.map(|m| format!(" ({m})")).unwrap_or_default()
                        ),
                    ));
                }
            }
        }
        Err(e) => items.push(HealthItem::new("database", "error", format!("Could not read accounts: {e}"))),
    }

    let failing: Vec<String> = sqlx::query_scalar("SELECT name FROM notifications WHERE enabled = 1 AND last_ok = 0")
        .fetch_all(&app.db)
        .await
        .unwrap_or_default();
    if !failing.is_empty() {
        items.push(HealthItem::new(
            "notifications",
            "warning",
            format!("Last delivery failed: {}", failing.join(", ")),
        ));
    }
    items
}

/// Periodically re-check; publish changes over SSE and notify about new problems
/// (`health`) and resolved ones (`healthRestored`).
pub async fn monitor(app: AppState) {
    let mut previous: Option<Vec<HealthItem>> = None;
    loop {
        let current = check(&app).await;
        if previous.as_ref() != Some(&current) {
            app.hub.publish("health", &current);
            if let Some(prev) = &previous {
                let before: HashSet<&HealthItem> = prev.iter().collect();
                let after: HashSet<&HealthItem> = current.iter().collect();
                for item in current.iter().filter(|i| !before.contains(i) && i.kind != "notice") {
                    // A failing notifier reporting its own failure would loop.
                    if item.source != "notifications" {
                        app.notifier.dispatch_raw(&app, "health", &item.message).await;
                    }
                }
                for item in prev.iter().filter(|i| !after.contains(i) && i.kind != "notice") {
                    if item.source != "notifications" && !current.iter().any(|c| c.source == item.source) {
                        app.notifier
                            .dispatch_raw(&app, "healthRestored", &format!("Resolved: {}", item.message))
                            .await;
                    }
                }
            }
            previous = Some(current);
        }
        tokio::select! {
            _ = app.shutdown.cancelled() => return,
            _ = tokio::time::sleep(MONITOR_INTERVAL) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writable_reports_missing_and_existing_dirs() {
        let dir = tempfile::tempdir().unwrap();
        assert!(writable(dir.path()).is_ok());
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none(), "probe file removed");
        assert!(writable(&dir.path().join("missing")).is_err());
    }
}
