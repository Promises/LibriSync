//! Library sync, per account (DESIGN.md "Sync").
//!
//! Pages are fetched with the app's `runPagedSync` policy
//! (`modules/expo-rust-bridge/index.ts`): three attempts per page with a growing
//! backoff, skip a page that still fails, stop after three consecutive failed pages,
//! never more than 500 pages. A provider ends its library with `has_more = false`
//! (Audible and Chirp only on an empty page — there is no total to check against).
//!
//! Every provider goes through `AnyProvider`; Audible's implementation runs
//! `ensure_valid_token` (30 min threshold, saving refreshed tokens to the core DB)
//! before `AudibleClient::sync_library_page`, exactly as the app's bridge does. The
//! account JSON is re-read from the DB for every page so a mid-sync refresh is used.

use std::collections::HashSet;
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use rand::Rng;
use rust_core::api::library::SyncStats;
use rust_core::providers::AnyProvider;
use rust_core::LibationError;

use crate::accounts::{self, CredentialError};
use crate::history::{self, NewEvent};
use crate::queue::{self, EnqueueError, Priority};
use crate::state::{App, AppState};
use crate::time;

/// The most books one sync queues as new purchases (see `enqueue_new_books`).
pub const MAX_NEW_PER_SYNC: usize = 10;
/// Successful but incomplete syncs after which the baseline is set anyway (see `finish`).
pub const BASELINE_ATTEMPTS: i64 = 3;

#[derive(Debug, Clone)]
pub struct PagePolicy {
    pub attempts: u32,
    pub max_consecutive_failures: u32,
    pub max_pages: i32,
    /// Wait after a failed attempt, multiplied by the attempt number.
    pub backoff: StdDuration,
}

impl Default for PagePolicy {
    fn default() -> Self {
        Self {
            attempts: 3,
            max_consecutive_failures: 3,
            max_pages: 500,
            backoff: StdDuration::from_secs(2),
        }
    }
}

#[derive(Debug)]
pub enum PageError {
    /// Worth another attempt (network, server error).
    Retryable(String),
    /// Retrying cannot help (credentials rejected): abandon the sync.
    Fatal(String),
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct PagedOutcome {
    pub pages_ok: u32,
    pub items: i64,
    pub added: i64,
    pub updated: i64,
    pub items_failed: i64,
    /// `(page, last error)` for pages skipped after every attempt failed.
    pub pages_skipped: Vec<(i32, String)>,
    /// Why pagination stopped before the library ended, if it did.
    pub stopped: Option<String>,
    /// A fatal (authentication) error.
    pub fatal: Option<String>,
}

impl PagedOutcome {
    pub fn ok(&self) -> bool {
        self.fatal.is_none() && self.stopped.is_none()
    }

    /// Every page was read and every item in it understood. Only such a sync may
    /// establish the baseline: a book on a skipped page, or one that couldn't be read,
    /// would otherwise show up on the next sync as a "new purchase".
    pub fn complete(&self) -> bool {
        self.ok() && self.pages_skipped.is_empty() && self.items_failed == 0
    }

    /// A one-line summary for `lastSyncResult.message`; `None` for a clean sync.
    pub fn message(&self) -> Option<String> {
        if let Some(f) = &self.fatal {
            return Some(f.clone());
        }
        let mut parts = Vec::new();
        if let Some(s) = &self.stopped {
            parts.push(s.clone());
        }
        if !self.pages_skipped.is_empty() {
            let last = &self.pages_skipped[self.pages_skipped.len() - 1];
            parts.push(format!(
                "{} page(s) skipped (page {}: {})",
                self.pages_skipped.len(),
                last.0,
                last.1
            ));
        }
        if self.items_failed > 0 {
            parts.push(format!("{} item(s) unreadable", self.items_failed));
        }
        (!parts.is_empty()).then(|| parts.join("; "))
    }
}

/// Drive a paged sync with the app's retry policy. `fetch(page)` syncs one 1-based
/// page; `sleep` is injected so tests don't wait.
pub async fn run_paged<F, Fut, S, SFut>(policy: &PagePolicy, mut fetch: F, mut sleep: S) -> PagedOutcome
where
    F: FnMut(i32) -> Fut,
    Fut: Future<Output = Result<SyncStats, PageError>>,
    S: FnMut(StdDuration) -> SFut,
    SFut: Future<Output = ()>,
{
    let mut out = PagedOutcome::default();
    let mut page = 1;
    let mut has_more = true;
    let mut consecutive_failures = 0;

    while has_more && page <= policy.max_pages {
        let mut stats = None;
        let mut last_error = String::new();
        for attempt in 1..=policy.attempts {
            match fetch(page).await {
                Ok(s) => {
                    stats = Some(s);
                    break;
                }
                Err(PageError::Fatal(msg)) => {
                    out.fatal = Some(msg);
                    return out;
                }
                Err(PageError::Retryable(msg)) => {
                    tracing::warn!("Sync page {page} attempt {attempt} failed: {msg}");
                    last_error = msg;
                    if attempt < policy.attempts {
                        sleep(policy.backoff * attempt).await;
                    }
                }
            }
        }

        let Some(stats) = stats else {
            consecutive_failures += 1;
            out.pages_skipped.push((page, last_error));
            if consecutive_failures >= policy.max_consecutive_failures {
                out.stopped = Some(format!(
                    "Stopped after {consecutive_failures} consecutive failed pages; the library may be incomplete"
                ));
                return out;
            }
            page += 1;
            continue;
        };

        consecutive_failures = 0;
        out.pages_ok += 1;
        out.items += stats.total_items as i64;
        out.added += stats.books_added as i64;
        out.updated += stats.books_updated as i64;
        out.items_failed += stats.items_failed as i64;
        has_more = stats.has_more;
        page += 1;
    }

    if has_more && page > policy.max_pages {
        out.stopped = Some(format!("Stopped at the {}-page limit", policy.max_pages));
    }
    out
}

/// Errors that mean the stored credential is no longer accepted.
pub fn is_auth_error(e: &LibationError) -> bool {
    if e.error_code() == "auth_expired" {
        return true;
    }
    // A bare 401 (Libro.fm's edge answers bad tokens this way) also means re-sign-in.
    matches!(
        e,
        LibationError::ApiRequestFailed { status_code: Some(401), .. }
            | LibationError::UnexpectedStatusCode { status_code: 401, .. }
    )
}

/// The next scheduled sync: `interval` from now, ±30 minutes of jitter so accounts
/// (and instances) don't all hit the stores at the same minute.
pub fn next_sync_at<R: Rng>(rng: &mut R, now: DateTime<Utc>, interval_hours: u32) -> DateTime<Utc> {
    let jitter = rng.gen_range(-1800..=1800);
    now + Duration::hours(interval_hours as i64) + Duration::seconds(jitter)
}

/// Accounts currently syncing.
#[derive(Default)]
pub struct Running(Mutex<HashSet<String>>);

pub struct RunningGuard<'a> {
    set: &'a Running,
    id: String,
}

impl Running {
    pub fn try_start(&self, id: &str) -> Option<RunningGuard<'_>> {
        let mut set = self.0.lock().ok()?;
        if !set.insert(id.to_string()) {
            return None;
        }
        Some(RunningGuard { set: self, id: id.to_string() })
    }

    pub fn is_running(&self, id: &str) -> bool {
        self.0.lock().map(|s| s.contains(id)).unwrap_or(false)
    }
}

impl Drop for RunningGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut set) = self.set.0.lock() {
            set.remove(&self.id);
        }
    }
}

/// Start a sync in the background (no-op if that account is already syncing).
pub fn spawn(app: AppState, id: String) {
    tokio::spawn(async move {
        if let Err(e) = sync_account(&app, &id).await {
            tracing::error!("Sync of account failed: {e:#}");
        }
    });
}

/// Sync one account now. Publishes the account (with `syncing`) when the sync starts
/// and again when it has finished.
pub async fn sync_account(app: &AppState, id: &str) -> anyhow::Result<()> {
    let result = {
        let Some(_guard) = app.syncs.try_start(id) else {
            tracing::debug!("Sync already running for an account; skipping");
            return Ok(());
        };
        accounts::publish(app, id).await;
        sync_locked(app, id).await
    };
    accounts::publish(app, id).await;
    result
}

async fn sync_locked(app: &AppState, id: &str) -> anyhow::Result<()> {
    accounts::ensure_state(app, id).await?;
    let Some(account) = accounts::load_json(app, id).await? else {
        return Ok(());
    };
    let provider = accounts::provider_of(&account)?;
    let started = time::now();
    // Books.created_at uses SQLite's CURRENT_TIMESTAMP form at second precision.
    let started_core = time::core_fmt(started);
    // The id, not the display name: Libro.fm and Chirp names are email addresses.
    tracing::info!("Syncing {} account {id}", provider.as_str());

    let outcome = run_paged(
        &PagePolicy::default(),
        |page| {
            let app = app.clone();
            let id = id.to_string();
            async move {
                // Refresh an expiring token under the account's lock, so a download
                // planning at the same moment doesn't refresh it a second time.
                let creds = match accounts::fresh_credentials(&app, &id).await {
                    Ok(Some(creds)) => creds,
                    Ok(None) => return Err(PageError::Fatal("The account was removed during the sync".into())),
                    Err(CredentialError::Refresh(e)) if is_auth_error(&e) => return Err(PageError::Fatal(e.to_string())),
                    Err(e) => return Err(PageError::Retryable(e.to_string())),
                };
                let p = AnyProvider::get(provider).map_err(|e| PageError::Fatal(e.to_string()))?;
                p.as_provider()
                    .sync_library_page(&app.core, &creds, page)
                    .await
                    .map_err(|e| {
                        if is_auth_error(&e) {
                            PageError::Fatal(e.to_string())
                        } else {
                            PageError::Retryable(e.to_string())
                        }
                    })
            }
        },
        tokio::time::sleep,
    )
    .await;

    finish(app, id, provider.as_str(), &started_core, &outcome).await
}

pub(crate) async fn finish(app: &AppState, id: &str, provider: &str, started_core: &str, outcome: &PagedOutcome) -> anyhow::Result<()> {
    let now = time::now();
    let settings = app.settings();
    let next = next_sync_at(&mut rand::thread_rng(), now, settings.sync.interval_hours);
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM core.BookAccounts WHERE account = ? AND is_deleted = 0")
        .bind(id)
        .fetch_one(&app.db)
        .await?;
    let ok = outcome.ok();
    let message = outcome.message();

    sqlx::query(
        "UPDATE account_state SET last_sync = ?, next_sync = ?, last_sync_ok = ?, last_sync_added = ?,
                last_sync_updated = ?, last_sync_total = ?, last_sync_message = ?
         WHERE account_id = ?",
    )
    .bind(time::fmt(now))
    .bind(time::fmt(next))
    .bind(ok)
    .bind(outcome.added)
    .bind(outcome.updated)
    .bind(total)
    .bind(&message)
    .bind(id)
    .execute(&app.db)
    .await?;

    if let Some(fatal) = &outcome.fatal {
        let was: Option<String> = sqlx::query_scalar("SELECT auth_status FROM account_state WHERE account_id = ?")
            .bind(id)
            .fetch_optional(&app.db)
            .await?;
        accounts::set_auth_status(app, id, "expired").await?;
        if was.as_deref() != Some("expired") {
            history::record(
                app,
                NewEvent::new("authExpired", format!("Sign-in for this {provider} account is no longer accepted: {fatal}"))
                    .account(id),
            )
            .await;
        }
        history::record(app, NewEvent::new("syncFailed", format!("Sync failed: {fatal}")).account(id)).await;
        return Ok(());
    }

    if outcome.pages_ok > 0 {
        accounts::set_auth_status(app, id, "ok").await?;
    }
    if ok {
        rust_core::storage::accounts::update_last_sync(app.core.pool(), id).await?;
    }

    let (baseline, attempts): (Option<String>, i64) =
        sqlx::query_as("SELECT baseline_at, baseline_attempts FROM account_state WHERE account_id = ?")
            .bind(id)
            .fetch_optional(&app.db)
            .await?
            .unwrap_or((None, 0));

    // Until a complete sync (no skipped pages, no unreadable items) has established the
    // baseline, everything found is "existing library" (back-fill), never "new". A
    // library with a page or item that fails every time would never get one, and with
    // no baseline nothing downloads at all; so after BASELINE_ATTEMPTS successful but
    // incomplete syncs the baseline is set anyway (books that turn up later are still
    // capped by MAX_NEW_PER_SYNC).
    let establishes_baseline = baseline.is_none()
        && (outcome.complete() || (ok && attempts + 1 >= BASELINE_ATTEMPTS));
    if baseline.is_none() {
        if establishes_baseline {
            sqlx::query("UPDATE account_state SET baseline_at = ?, baseline_attempts = baseline_attempts + 1 WHERE account_id = ?")
                .bind(time::fmt(now))
                .bind(id)
                .execute(&app.db)
                .await?;
            if !outcome.complete() {
                let note = format!(
                    "Library baseline set although {} sync(s) in a row were incomplete ({}); books that \
                     appear later are treated as new purchases",
                    attempts + 1,
                    message.as_deref().unwrap_or("incomplete")
                );
                tracing::warn!("Account {id}: {note}");
                history::record(app, NewEvent::new("syncCompleted", note).account(id)).await;
            }
        } else if ok {
            sqlx::query("UPDATE account_state SET baseline_attempts = baseline_attempts + 1 WHERE account_id = ?")
                .bind(id)
                .execute(&app.db)
                .await?;
        }
    } else {
        enqueue_new_books(app, id, started_core).await?;
    }

    let summary = format!(
        "Synced {} item(s): {} added, {} updated{}",
        outcome.items,
        outcome.added,
        outcome.updated,
        message.as_deref().map(|m| format!(" ({m})")).unwrap_or_default()
    );
    let data = serde_json::json!({
        "added": outcome.added, "updated": outcome.updated, "items": outcome.items,
        "itemsFailed": outcome.items_failed, "pagesSkipped": outcome.pages_skipped.len(),
        "baseline": establishes_baseline,
    });
    let event = if ok { "syncCompleted" } else { "syncFailed" };
    history::record(app, NewEvent::new(event, summary).account(id).data(data)).await;

    // A first sync makes the whole library eligible for back-fill.
    let app2 = app.clone();
    tokio::spawn(async move { crate::backfill::run_once(&app2).await });
    Ok(())
}

/// Books first seen by this sync are new purchases: record them and queue them at
/// `new` priority when the account wants that.
///
/// At most [`MAX_NEW_PER_SYNC`] of them, the most recently purchased: a sync that finds
/// more than that is almost certainly seeing library the baseline missed (a store
/// hiccup, a page that came back this time), not a shopping spree, and queueing it all
/// at `new` priority would skip the back-fill pacing. The rest are left to back-fill.
async fn enqueue_new_books(app: &App, id: &str, started_core: &str) -> anyhow::Result<()> {
    let new_books: Vec<(String, String)> = sqlx::query_as(
        "SELECT b.audible_product_id, b.title FROM core.Books b
         JOIN core.BookAccounts ba ON ba.book_id = b.book_id
         WHERE ba.account = ? AND ba.is_deleted = 0 AND b.created_at >= ? AND b.content_type != 4
         ORDER BY ba.date_added DESC, b.created_at DESC, b.book_id",
    )
    .bind(id)
    .bind(started_core)
    .fetch_all(&app.db)
    .await?;
    if new_books.is_empty() {
        return Ok(());
    }
    let (enabled, auto): (bool, bool) =
        sqlx::query_as("SELECT enabled, auto_download_new FROM account_state WHERE account_id = ?")
            .bind(id)
            .fetch_optional(&app.db)
            .await?
            .unwrap_or((true, true));

    let excess = new_books.len().saturating_sub(MAX_NEW_PER_SYNC);
    if excess > 0 {
        let message = format!(
            "{} books appeared in one sync; treating the {MAX_NEW_PER_SYNC} most recent as new purchases and \
             leaving {excess} to back-fill",
            new_books.len()
        );
        tracing::warn!("Account {id}: {message}");
        history::record(
            app,
            NewEvent::new("syncCompleted", message)
                .account(id)
                .data(serde_json::json!({ "newBooks": new_books.len(), "treatedAsBackfill": excess })),
        )
        .await;
    }

    for (book_id, title) in new_books.into_iter().take(MAX_NEW_PER_SYNC) {
        history::record(app, NewEvent::new("bookAdded", format!("New in library: {title}")).account(id).book(&book_id, &title)).await;
        if enabled && auto {
            match queue::enqueue(app, &book_id, Priority::New, false).await {
                Ok(_) | Err(EnqueueError::AlreadyDownloaded) | Err(EnqueueError::Skipped) => {}
                Err(EnqueueError::PlusExcluded) => {
                    tracing::info!("Not queueing {title}: Audible Plus catalogue titles are excluded");
                }
                Err(e) => tracing::warn!("Could not queue new book {book_id}: {e:?}"),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    fn stats(items: i32, has_more: bool) -> SyncStats {
        SyncStats {
            total_items: items,
            total_library_count: 0,
            books_added: items,
            books_updated: 0,
            books_absent: 0,
            errors: Vec::new(),
            items_failed: 0,
            has_more,
        }
    }

    /// A fake provider: `script[page]` is the list of results for successive attempts
    /// at that page (the last one repeats).
    async fn drive(script: HashMap<i32, Vec<Result<SyncStats, &'static str>>>, fatal_on: Option<i32>) -> (PagedOutcome, Vec<i32>, Vec<StdDuration>) {
        let calls = RefCell::new(Vec::new());
        let sleeps = RefCell::new(Vec::new());
        let attempts = RefCell::new(HashMap::<i32, usize>::new());
        let policy = PagePolicy { backoff: StdDuration::from_secs(1), ..Default::default() };
        let out = run_paged(
            &policy,
            |page| {
                calls.borrow_mut().push(page);
                let n = {
                    let mut a = attempts.borrow_mut();
                    let e = a.entry(page).or_insert(0);
                    *e += 1;
                    *e - 1
                };
                let result = if fatal_on == Some(page) {
                    Err(PageError::Fatal("token rejected".into()))
                } else {
                    let list = script.get(&page).cloned().unwrap_or_else(|| vec![Ok(stats(0, false))]);
                    match list.get(n).or(list.last()).cloned().unwrap_or(Ok(stats(0, false))) {
                        Ok(s) => Ok(s),
                        Err(m) => Err(PageError::Retryable(m.to_string())),
                    }
                };
                async move { result }
            },
            |d| {
                sleeps.borrow_mut().push(d);
                async {}
            },
        )
        .await;
        (out, calls.into_inner(), sleeps.into_inner())
    }

    #[tokio::test]
    async fn walks_pages_until_the_provider_says_done() {
        let script = HashMap::from([
            (1, vec![Ok(stats(250, true))]),
            (2, vec![Ok(stats(10, true))]),
            (3, vec![Ok(stats(0, false))]),
        ]);
        let (out, calls, sleeps) = drive(script, None).await;
        assert_eq!(calls, vec![1, 2, 3]);
        assert!(sleeps.is_empty());
        assert!(out.ok());
        assert_eq!((out.pages_ok, out.items, out.added), (3, 260, 260));
        assert_eq!(out.message(), None);
    }

    #[tokio::test]
    async fn retries_a_page_with_growing_backoff() {
        let script = HashMap::from([
            (1, vec![Err("reset"), Err("reset"), Ok(stats(5, false))]),
        ]);
        let (out, calls, sleeps) = drive(script, None).await;
        assert_eq!(calls, vec![1, 1, 1]);
        assert_eq!(sleeps, vec![StdDuration::from_secs(1), StdDuration::from_secs(2)]);
        assert!(out.ok());
        assert_eq!(out.items, 5);
    }

    #[tokio::test]
    async fn skips_a_page_that_keeps_failing_and_continues() {
        let script = HashMap::from([
            (1, vec![Ok(stats(250, true))]),
            (2, vec![Err("boom")]),
            (3, vec![Ok(stats(7, false))]),
        ]);
        let (out, calls, _) = drive(script, None).await;
        assert_eq!(calls, vec![1, 2, 2, 2, 3]);
        assert!(out.ok(), "a skipped page doesn't fail the sync");
        assert_eq!(out.pages_skipped, vec![(2, "boom".to_string())]);
        assert_eq!(out.items, 257);
        assert!(out.message().unwrap().contains("1 page(s) skipped"));
    }

    #[tokio::test]
    async fn stops_after_three_consecutive_failed_pages() {
        let script = HashMap::from([
            (1, vec![Ok(stats(250, true))]),
            (2, vec![Err("down")]),
            (3, vec![Err("down")]),
            (4, vec![Err("down")]),
            (5, vec![Ok(stats(1, false))]),
        ]);
        let (out, calls, _) = drive(script, None).await;
        assert_eq!(calls.iter().filter(|p| **p == 5).count(), 0);
        assert_eq!(out.pages_skipped.len(), 3);
        assert!(!out.ok());
        assert!(out.stopped.as_deref().unwrap().contains("3 consecutive"));
    }

    #[tokio::test]
    async fn an_auth_error_ends_the_sync_without_retrying() {
        let script = HashMap::from([(1, vec![Ok(stats(250, true))])]);
        let (out, calls, sleeps) = drive(script, Some(2)).await;
        assert_eq!(calls, vec![1, 2]);
        assert!(sleeps.is_empty());
        assert_eq!(out.fatal.as_deref(), Some("token rejected"));
        assert!(!out.ok());
    }

    #[tokio::test]
    async fn caps_at_the_page_limit() {
        let calls = RefCell::new(0);
        let policy = PagePolicy { max_pages: 4, ..Default::default() };
        let out = run_paged(
            &policy,
            |_| {
                *calls.borrow_mut() += 1;
                async { Ok(stats(1, true)) }
            },
            |_| async {},
        )
        .await;
        assert_eq!(*calls.borrow(), 4);
        assert!(out.stopped.as_deref().unwrap().contains("4-page limit"));
    }

    #[test]
    fn jitter_stays_within_half_an_hour() {
        let mut rng = rand::thread_rng();
        let now = time::now();
        for _ in 0..200 {
            let n = next_sync_at(&mut rng, now, 24);
            let d = n - now;
            assert!(d >= Duration::minutes(24 * 60 - 30) && d <= Duration::minutes(24 * 60 + 30));
        }
    }

    #[test]
    fn auth_errors_are_recognised() {
        assert!(is_auth_error(&LibationError::AuthenticationFailed { message: "x".into(), account_id: None }));
        assert!(is_auth_error(&LibationError::TokenExpired));
        assert!(!is_auth_error(&LibationError::InvalidInput("x".into())));
    }
}
