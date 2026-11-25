//! Accounts: credentials live in the core DB (`core.Accounts`, written only through
//! `rust_core::storage::accounts`), server-side state in `account_state`.
//!
//! Accounts are provider-scoped: every path that hands an account to provider code
//! reads its `provider` tag and dispatches on it.

use chrono::{DateTime, Utc};
use rust_core::providers::ProviderId;
use serde::Serialize;
use sqlx::SqlitePool;

use crate::pacing::AccountPacing;
use crate::state::App;
use crate::time;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    pub ok: bool,
    pub added: i64,
    pub updated: i64,
    pub total: i64,
    pub message: Option<String>,
}

/// `AccountView` in API.md.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub id: String,
    pub provider: String,
    pub name: String,
    pub locale: Option<String>,
    pub enabled: bool,
    pub auto_download_new: bool,
    pub backfill: bool,
    pub last_sync: Option<String>,
    pub next_sync: Option<String>,
    pub last_sync_result: Option<SyncResult>,
    pub book_count: i64,
    pub downloaded_count: i64,
    pub cooldown_until: Option<String>,
    pub cooldown_reason: Option<String>,
    pub licences_last24h: i64,
    pub daily_licence_cap: Option<u32>,
    pub auth_status: String,
    pub syncing: bool,
}

#[derive(sqlx::FromRow)]
struct ViewRow {
    account_id: String,
    provider: String,
    account_name: String,
    locale_code: String,
    enabled: bool,
    auto_download_new: bool,
    backfill: bool,
    last_sync: Option<String>,
    next_sync: Option<String>,
    last_sync_ok: Option<bool>,
    last_sync_added: Option<i64>,
    last_sync_updated: Option<i64>,
    last_sync_total: Option<i64>,
    last_sync_message: Option<String>,
    cooldown_until: Option<String>,
    cooldown_reason: Option<String>,
    auth_status: String,
    book_count: i64,
    downloaded_count: i64,
    licences: i64,
}

const VIEW_SQL: &str = "
SELECT a.account_id,
       COALESCE(a.provider, 'audible') AS provider,
       a.account_name,
       a.locale_code,
       COALESCE(s.enabled, 1) AS enabled,
       COALESCE(s.auto_download_new, 1) AS auto_download_new,
       COALESCE(s.backfill, 1) AS backfill,
       s.last_sync, s.next_sync, s.last_sync_ok, s.last_sync_added, s.last_sync_updated,
       s.last_sync_total, s.last_sync_message,
       s.cooldown_until, s.cooldown_reason,
       COALESCE(s.auth_status, 'unknown') AS auth_status,
       (SELECT COUNT(*) FROM core.BookAccounts ba WHERE ba.account = a.account_id AND ba.is_deleted = 0) AS book_count,
       (SELECT COUNT(*) FROM core.BookAccounts ba
          JOIN core.Books b ON b.book_id = ba.book_id
          JOIN downloads d ON d.book_id = b.audible_product_id
         WHERE ba.account = a.account_id AND ba.is_deleted = 0) AS downloaded_count,
       (SELECT COUNT(*) FROM licence_requests l WHERE l.account_id = a.account_id AND l.time > ?) AS licences
FROM core.Accounts a
LEFT JOIN account_state s ON s.account_id = a.account_id";

fn to_view(r: ViewRow, cap: u32, now: DateTime<Utc>, syncing: bool) -> AccountView {
    let audible = r.provider == "audible";
    let cooldown_active = r
        .cooldown_until
        .as_deref()
        .and_then(time::parse)
        .is_some_and(|u| u > now);
    AccountView {
        id: r.account_id,
        name: r.account_name,
        locale: audible.then_some(r.locale_code),
        enabled: r.enabled,
        auto_download_new: r.auto_download_new,
        backfill: r.backfill,
        last_sync: r.last_sync,
        next_sync: r.next_sync,
        last_sync_result: r.last_sync_ok.map(|ok| SyncResult {
            ok,
            added: r.last_sync_added.unwrap_or(0),
            updated: r.last_sync_updated.unwrap_or(0),
            total: r.last_sync_total.unwrap_or(0),
            message: r.last_sync_message,
        }),
        book_count: r.book_count,
        downloaded_count: r.downloaded_count,
        cooldown_until: if cooldown_active { r.cooldown_until } else { None },
        cooldown_reason: if cooldown_active { r.cooldown_reason } else { None },
        licences_last24h: if audible { r.licences } else { 0 },
        daily_licence_cap: audible.then_some(cap),
        auth_status: r.auth_status,
        provider: r.provider,
        syncing,
    }
}

pub async fn list(app: &App) -> sqlx::Result<Vec<AccountView>> {
    let now = time::now();
    let cap = app.settings().audible.daily_licence_cap;
    let rows: Vec<ViewRow> = sqlx::query_as(&format!("{VIEW_SQL} ORDER BY provider, a.account_name"))
        .bind(time::fmt(now - chrono::Duration::hours(24)))
        .fetch_all(&app.db)
        .await?;
    Ok(rows.into_iter().map(|r| {
        let syncing = app.syncs.is_running(&r.account_id);
        to_view(r, cap, now, syncing)
    }).collect())
}

pub async fn get(app: &App, id: &str) -> sqlx::Result<Option<AccountView>> {
    let now = time::now();
    let cap = app.settings().audible.daily_licence_cap;
    let row: Option<ViewRow> = sqlx::query_as(&format!("{VIEW_SQL} WHERE a.account_id = ?"))
        .bind(time::fmt(now - chrono::Duration::hours(24)))
        .bind(id)
        .fetch_optional(&app.db)
        .await?;
    Ok(row.map(|r| {
        let syncing = app.syncs.is_running(&r.account_id);
        to_view(r, cap, now, syncing)
    }))
}

/// Publish an account's current view over SSE.
pub async fn publish(app: &App, id: &str) {
    match get(app, id).await {
        Ok(Some(view)) => app.hub.publish("account", &view),
        Ok(None) => {}
        Err(e) => tracing::warn!("Could not read account for an update event: {e}"),
    }
}

/// Make sure `account_state` has a row, seeded from the global defaults.
pub async fn ensure_state(app: &App, id: &str) -> sqlx::Result<()> {
    let s = app.settings();
    sqlx::query(
        "INSERT INTO account_state (account_id, enabled, auto_download_new, backfill) VALUES (?, 1, ?, ?)
         ON CONFLICT(account_id) DO NOTHING",
    )
    .bind(id)
    .bind(s.sync.auto_download_new)
    .bind(s.backfill.enabled)
    .execute(&app.db)
    .await?;
    Ok(())
}

/// The stored account JSON (rust-core's shape), which is also the provider credential blob.
pub async fn load_json(app: &App, id: &str) -> anyhow::Result<Option<serde_json::Value>> {
    match rust_core::storage::accounts::get_account(app.core.pool(), id).await? {
        Some(json) => Ok(Some(serde_json::from_str(&json)?)),
        None => Ok(None),
    }
}

pub fn provider_of(account: &serde_json::Value) -> anyhow::Result<ProviderId> {
    let p = account.get("provider").and_then(|p| p.as_str()).unwrap_or("audible");
    Ok(ProviderId::from_str(p)?)
}

/// Accounts and the state the scheduler needs.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AccountSched {
    pub account_id: String,
    pub provider: String,
    pub enabled: bool,
    pub backfill: bool,
    pub baseline_at: Option<String>,
    pub next_sync: Option<String>,
    pub auth_status: String,
    pub cooldown_until: Option<String>,
}

pub async fn list_sched(pool: &SqlitePool) -> sqlx::Result<Vec<AccountSched>> {
    sqlx::query_as(
        "SELECT a.account_id, COALESCE(a.provider, 'audible') AS provider,
                COALESCE(s.enabled, 1) AS enabled,
                COALESCE(s.backfill, 1) AS backfill, s.baseline_at, s.next_sync,
                COALESCE(s.auth_status, 'unknown') AS auth_status, s.cooldown_until
         FROM core.Accounts a LEFT JOIN account_state s ON s.account_id = a.account_id
         ORDER BY a.account_id",
    )
    .fetch_all(pool)
    .await
}

/// The pacing state of an Audible account (without the in-flight flag).
pub async fn pacing(pool: &SqlitePool, id: &str, now: DateTime<Utc>) -> sqlx::Result<AccountPacing> {
    let row: Option<(Option<String>, i64, Option<String>)> = sqlx::query_as(
        "SELECT cooldown_until, cooldown_level, next_licence_at FROM account_state WHERE account_id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    let recent: Vec<String> =
        sqlx::query_scalar("SELECT time FROM licence_requests WHERE account_id = ? AND time > ?")
            .bind(id)
            .bind(time::fmt(now - crate::pacing::CAP_WINDOW))
            .fetch_all(pool)
            .await?;
    let (cooldown_until, level, next) = row.unwrap_or((None, 0, None));
    Ok(AccountPacing {
        cooldown_until: cooldown_until.as_deref().and_then(time::parse),
        cooldown_level: level.max(0) as u32,
        next_licence_at: next.as_deref().and_then(time::parse),
        recent_requests: recent.iter().filter_map(|t| time::parse(t)).collect(),
        in_flight: false,
    })
}

pub async fn set_auth_status(app: &App, id: &str, status: &str) -> sqlx::Result<()> {
    ensure_state(app, id).await?;
    sqlx::query("UPDATE account_state SET auth_status = ? WHERE account_id = ?")
        .bind(status)
        .bind(id)
        .execute(&app.db)
        .await?;
    Ok(())
}

pub async fn clear_cooldown(app: &App, id: &str) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE account_state SET cooldown_until = NULL, cooldown_reason = NULL, cooldown_level = 0 WHERE account_id = ?",
    )
    .bind(id)
    .execute(&app.db)
    .await?;
    app.queue.wake();
    Ok(())
}

/// Save a freshly signed-in (or re-signed-in) account and start its first sync.
pub async fn add(app: &crate::state::AppState, account_id: &str, account_json: &serde_json::Value) -> anyhow::Result<AccountView> {
    let existed = rust_core::storage::accounts::get_account(app.core.pool(), account_id)
        .await?
        .is_some();
    let json = serde_json::to_string(account_json)?;
    rust_core::storage::accounts::save_account(app.core.pool(), account_id, &json).await?;
    ensure_state(app, account_id).await?;
    set_auth_status(app, account_id, "ok").await?;

    let provider = provider_of(account_json)?;
    let name = account_json
        .get("account_name")
        .and_then(|n| n.as_str())
        .unwrap_or(account_id)
        .to_string();
    if !existed {
        crate::history::record(
            app,
            crate::history::NewEvent::new("accountAdded", format!("Added {} account {name}", provider.as_str()))
                .account(account_id),
        )
        .await;
    }
    crate::sync::spawn(app.clone(), account_id.to_string());
    app.queue.wake();

    let view = get(app, account_id).await?.ok_or_else(|| anyhow::anyhow!("account vanished after saving"))?;
    app.hub.publish("account", &view);
    Ok(view)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(provider: &str) -> ViewRow {
        ViewRow {
            account_id: "a".into(),
            provider: provider.into(),
            account_name: "A".into(),
            locale_code: "us".into(),
            enabled: true,
            auto_download_new: true,
            backfill: false,
            last_sync: None,
            next_sync: None,
            last_sync_ok: None,
            last_sync_added: None,
            last_sync_updated: None,
            last_sync_total: None,
            last_sync_message: None,
            cooldown_until: Some("2000-01-01T00:00:00.000Z".into()),
            cooldown_reason: Some("old".into()),
            auth_status: "ok".into(),
            book_count: 3,
            downloaded_count: 1,
            licences: 4,
        }
    }

    #[test]
    fn drm_free_accounts_have_no_locale_or_cap() {
        let v = to_view(row("chirp"), 20, time::now(), false);
        assert_eq!(v.locale, None);
        assert_eq!(v.daily_licence_cap, None);
        assert_eq!(v.licences_last24h, 0);
        let v = to_view(row("audible"), 20, time::now(), false);
        assert_eq!(v.locale.as_deref(), Some("us"));
        assert_eq!(v.daily_licence_cap, Some(20));
        assert_eq!(v.licences_last24h, 4);
    }

    #[test]
    fn an_expired_cooldown_is_not_reported() {
        let v = to_view(row("audible"), 20, time::now(), false);
        assert_eq!(v.cooldown_until, None);
        assert_eq!(v.cooldown_reason, None);
        let json = serde_json::to_value(&v).unwrap();
        assert!(json.get("licencesLast24h").is_some());
        assert!(json.get("lastSyncResult").is_some());
    }
}
