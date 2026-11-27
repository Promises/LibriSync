//! Back-fill: slowly download the library an account already had at its baseline.
//!
//! Audible accounts get **one** back-fill item in the queue at a time, so the licence
//! pacing in the dispatcher sees a single candidate and its `waitingReason` tells the
//! user when the next slot is. DRM-free stores have no licence budget and keep a small
//! queue depth instead.

use chrono::Duration;
use serde::Serialize;
use sqlx::SqlitePool;

use crate::accounts;
use crate::pacing::{self, PacingConfig};
use crate::queue::{self, EnqueueError, Priority, SLOT_STATES_SQL};
use crate::settings::BackfillOrder;
use crate::state::App;
use crate::time;

/// Back-fill items kept queued per DRM-free account.
const DRM_FREE_DEPTH: i64 = 3;
/// A book the user cancelled is left alone by back-fill for this long.
pub const CANCELLED_GRACE: Duration = Duration::days(7);

/// The conditions that make an account's book "wanted": owned, not downloaded, not
/// queued or failed, not recently cancelled, not skipped, not in title backoff, and not
/// an Audible Plus catalogue title while those are excluded.
/// Binds: ?1 account, ?2 provider, ?3 cancelled-grace cutoff, ?4 now, ?5 include Plus.
fn wanted_from_where() -> String {
    format!(
        "FROM core.BookAccounts ba
         JOIN core.Books b ON b.book_id = ba.book_id
         WHERE ba.account = ?1 AND ba.is_deleted = 0 AND COALESCE(b.source, 'audible') = ?2
           AND b.is_downloadable = 1 AND b.content_type != 4
           AND (b.is_ayce = 0 OR ?5 = 1)
           AND NOT EXISTS (SELECT 1 FROM downloads d WHERE d.book_id = b.audible_product_id)
           AND NOT EXISTS (SELECT 1 FROM queue q WHERE q.book_id = b.audible_product_id
                           AND (q.state IN {SLOT_STATES_SQL} OR (q.state = 'cancelled' AND q.finished_at > ?3)))
           AND NOT EXISTS (SELECT 1 FROM skips s WHERE s.book_id = b.audible_product_id)
           AND NOT EXISTS (SELECT 1 FROM title_backoff t WHERE t.book_id = b.audible_product_id AND t.until > ?4)"
    )
}

/// Wanted books for an account, best first by purchase date. `include_plus` is
/// `audible.includePlusCatalog`.
pub async fn wanted_for_account(
    pool: &SqlitePool,
    account_id: &str,
    provider: &str,
    order: BackfillOrder,
    include_plus: bool,
    limit: i64,
) -> sqlx::Result<Vec<String>> {
    let now = time::now();
    let dir = match order {
        BackfillOrder::NewestFirst => "DESC",
        BackfillOrder::OldestFirst => "ASC",
    };
    let sql = format!(
        "SELECT b.audible_product_id {} ORDER BY ba.date_added {dir}, b.book_id {dir} LIMIT ?6",
        wanted_from_where()
    );
    sqlx::query_scalar(&sql)
        .bind(account_id)
        .bind(provider)
        .bind(time::fmt(now - CANCELLED_GRACE))
        .bind(time::fmt(now))
        .bind(include_plus)
        .bind(limit)
        .fetch_all(pool)
        .await
}

async fn count_wanted(pool: &SqlitePool, account_id: &str, provider: &str, include_plus: bool) -> sqlx::Result<i64> {
    let now = time::now();
    sqlx::query_scalar(&format!("SELECT COUNT(*) {}", wanted_from_where()))
        .bind(account_id)
        .bind(provider)
        .bind(time::fmt(now - CANCELLED_GRACE))
        .bind(time::fmt(now))
        .bind(include_plus)
        .fetch_one(pool)
        .await
}

/// One back-fill pass over every account.
pub async fn run_once(app: &App) {
    if let Err(e) = run_inner(app).await {
        tracing::error!("Back-fill pass failed: {e:#}");
    }
}

/// While Audible Plus titles are excluded, take any automatically queued ones that
/// haven't started (queued before the switch was turned off, or before it existed)
/// out of the queue. Manual items stay: those were the user's explicit choice.
pub async fn drop_excluded_plus(app: &App) -> sqlx::Result<()> {
    if app.settings().audible.include_plus_catalog {
        return Ok(());
    }
    let removed: Vec<(i64, String)> = sqlx::query_as(
        "DELETE FROM queue WHERE state IN ('queued','waiting') AND priority != ?
           AND book_id IN (SELECT audible_product_id FROM core.Books WHERE is_ayce = 1)
         RETURNING id, title",
    )
    .bind(Priority::Manual.as_i64())
    .fetch_all(&app.db)
    .await?;
    for (id, title) in removed {
        tracing::info!("Removed {title} from the queue: Audible Plus catalogue titles are excluded");
        app.hub.publish("queueRemoved", &serde_json::json!({ "id": id.to_string() }));
    }
    Ok(())
}

async fn run_inner(app: &App) -> anyhow::Result<()> {
    drop_excluded_plus(app).await?;
    let settings = app.settings();
    if !settings.backfill.enabled {
        return Ok(());
    }
    let now = time::now();
    for acct in accounts::list_sched(&app.db).await? {
        if !acct.enabled || !acct.backfill || acct.baseline_at.is_none() || acct.auth_status == "expired" {
            continue;
        }
        let audible = acct.provider == "audible";
        if audible
            && acct
                .cooldown_until
                .as_deref()
                .and_then(time::parse)
                .is_some_and(|u| u > now)
        {
            continue;
        }
        let active: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM queue WHERE account_id = ? AND priority = ? AND state IN ('queued','waiting','planning','running')",
        )
        .bind(&acct.account_id)
        .bind(Priority::Backfill.as_i64())
        .fetch_one(&app.db)
        .await?;
        let depth = if audible { 1 } else { DRM_FREE_DEPTH };
        let room = depth - active;
        if room <= 0 {
            continue;
        }
        // Ask for a few extra: an enqueue can still be refused (e.g. the book's other
        // owner is preferred, or it was skipped a moment ago).
        let candidates = wanted_for_account(
            &app.db,
            &acct.account_id,
            &acct.provider,
            settings.backfill.order,
            settings.audible.include_plus_catalog,
            room + 5,
        )
        .await?;
        let mut added = 0;
        for book_id in candidates {
            if added >= room {
                break;
            }
            match queue::enqueue(app, &book_id, Priority::Backfill, false).await {
                Ok(_) => added += 1,
                Err(EnqueueError::Db(e)) => return Err(e.into()),
                Err(e) => tracing::debug!("Back-fill skipped {book_id}: {}", e.reason()),
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Estimate {
    pub per_day: f64,
    pub days_remaining: Option<f64>,
}

/// How fast the Audible back-fill drains: the sum over back-filling Audible accounts
/// of the licences pacing allows per day, against the Audible books still wanted.
pub async fn estimate(app: &App) -> sqlx::Result<Estimate> {
    let settings = app.settings();
    let cfg = PacingConfig {
        daily_cap: settings.audible.daily_licence_cap,
        spacing_min_minutes: settings.audible.spacing_min_minutes,
        spacing_max_minutes: settings.audible.spacing_max_minutes,
    };
    let per_account = pacing::backfill_per_day(&cfg, settings.backfill_window());
    let mut per_day = 0.0;
    let mut remaining = 0i64;
    if settings.backfill.enabled {
        for acct in accounts::list_sched(&app.db).await? {
            if acct.provider != "audible" || !acct.enabled || !acct.backfill {
                continue;
            }
            per_day += per_account;
            remaining +=
                count_wanted(&app.db, &acct.account_id, "audible", settings.audible.include_plus_catalog).await?;
        }
    }
    let per_day = (per_day * 10.0).round() / 10.0;
    let days_remaining = (per_day > 0.0).then(|| ((remaining as f64 / per_day) * 10.0).round() / 10.0);
    Ok(Estimate { per_day, days_remaining })
}
