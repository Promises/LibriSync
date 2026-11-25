//! One queue item, start to finish: plan (for Audible, the licence request) →
//! pipeline → import, and the state transition that follows from the outcome.
//!
//! Ordering rule for Audible: whatever a licence request means for the customer
//! (cooldown, "unavailable" hold, granted probe, expired sign-in) is persisted
//! **before** the in-flight mark is cleared, so the dispatcher can never start the
//! next licence request against state that doesn't show the last one's outcome yet.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Utc};
use rust_core::pipeline::{FailureKind, OutputFormat, PipelineError, PipelineJob, PipelineOutput, Progress, ProgressFn};
use rust_core::providers::{DownloadPlan, ProviderId};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::accounts::{self, CredentialError};
use crate::books;
use crate::executor::{classify, PlanFailure, PlanFailureKind};
use crate::history::{self, NewEvent};
use crate::pacing;
use crate::queue::{self, HoldKind, QueueRow, SLOT_STATES_SQL};
use crate::settings::Format;
use crate::state::{App, AppState};
use crate::template;
use crate::time;

/// Pause before re-running the pipeline with the same plan after a network or HTTP
/// blip. The plan's signed URLs are still valid, so this costs no licence.
const INLINE_RETRY_DELAY: StdDuration = if cfg!(test) { StdDuration::ZERO } else { StdDuration::from_secs(15) };

/// The states a worker's item is in while the worker owns it.
const ACTIVE_STATES_SQL: &str = "('planning','running')";

/// How an attempt ended.
#[derive(Debug)]
pub enum Outcome {
    Done { output: PipelineOutput, rel_path: String, format: Format },
    /// Audible throttled the customer; the cooldown is already recorded.
    Throttled(String),
    Denied(String),
    AuthExpired(String),
    /// Audible's licence endpoint answered with a server or network error; the
    /// customer's short hold is already recorded.
    Unavailable { message: String, until: DateTime<Utc> },
    /// A local problem every download would hit (disk full, ffmpeg, unwritable
    /// folder): fail this item and pause the queue.
    Halt { kind: HoldKind, message: String },
    Failed { message: String, retryable: bool },
    Cancelled,
}

/// Run queue item `id`. `licence_key` is the pacing key whose in-flight mark the
/// dispatcher set for this item (Audible only); `generation` is its registration.
pub async fn run(
    app: AppState,
    id: i64,
    generation: u64,
    licence_key: Option<String>,
    cancel: CancellationToken,
    user_cancelled: Arc<AtomicBool>,
) {
    let release = |app: &AppState| {
        if let Some(key) = &licence_key {
            app.queue.clear_licence_in_flight(key, id);
        }
        app.queue.unregister(id, generation);
        app.queue.wake();
    };
    let row = match queue::get(&app.db, id).await {
        Ok(Some(row)) => row,
        Ok(None) => return release(&app),
        Err(e) => {
            tracing::error!("Queue item {id} vanished: {e}");
            return release(&app);
        }
    };

    // The attempt runs in its own task so that a panic anywhere below (a provider, the
    // pipeline) fails this item instead of leaving it "planning" forever.
    let attempt = {
        let app = app.clone();
        let row = row.clone();
        let cancel = cancel.clone();
        let key = licence_key.clone();
        tokio::spawn(async move { execute(&app, &row, key.as_deref(), &cancel).await })
    };
    let outcome = match attempt.await {
        Ok(o) => o,
        Err(e) => Outcome::Failed { message: format!("Internal error: {e}"), retryable: true },
    };

    let user_cancel = user_cancelled.load(Ordering::SeqCst);
    if let Err(e) = finish(&app, &row, outcome, user_cancel).await {
        tracing::error!("Could not record the result of queue item {id}: {e:#}");
    }
    // Usually cleared right after planning; this covers the paths that end earlier.
    release(&app);
}

async fn execute(app: &AppState, row: &QueueRow, licence_key: Option<&str>, cancel: &CancellationToken) -> Outcome {
    match execute_inner(app, row, licence_key, cancel).await {
        Ok(o) => o,
        Err(e) => Outcome::Failed { message: format!("{e:#}"), retryable: true },
    }
}

async fn execute_inner(
    app: &AppState,
    row: &QueueRow,
    licence_key: Option<&str>,
    cancel: &CancellationToken,
) -> anyhow::Result<Outcome> {
    let settings = app.settings();
    let account = match accounts::fresh_credentials(app, &row.account_id).await {
        Ok(Some(account)) => account,
        Ok(None) => {
            return Ok(Outcome::Failed {
                message: "The account that owns this book has been removed".into(),
                retryable: false,
            })
        }
        Err(CredentialError::Db(e)) => return Err(e),
        Err(CredentialError::Refresh(e)) => {
            let failure = PlanFailure { kind: classify(&e), message: format!("Refreshing the sign-in failed: {e}") };
            return Ok(match failure.kind {
                PlanFailureKind::AuthExpired => {
                    accounts::set_auth_status(app, &row.account_id, "expired").await?;
                    Outcome::AuthExpired(failure.message)
                }
                _ => Outcome::Failed { message: failure.message, retryable: true },
            });
        }
    };
    let provider = accounts::provider_of(&account)?;
    let Some((meta, source)) = books::meta(&app.db, &row.book_id).await? else {
        return Ok(Outcome::Failed { message: "The book is no longer in the library".into(), retryable: false });
    };

    let format = settings.media.format;
    let options = match provider {
        ProviderId::Librofm => json!({ "format": format.as_str() }),
        _ => json!({}),
    };

    // --- Plan (Audible: the licence request, which is what pacing budgets) ---
    // Every Audible plan is a licence request and is recorded as one, even in the
    // (unexpected) case that the dispatcher passed no pacing key.
    let licence = match provider {
        ProviderId::Audible => {
            let key = match licence_key {
                Some(key) => key.to_string(),
                None => accounts::ensure_state(app, &row.account_id).await?,
            };
            let id = record_licence_request(app, row, &key).await?;
            Some((key, id))
        }
        _ => None,
    };
    let planned = tokio::select! {
        r = app.executor.plan(&app.core, provider, &account, &row.book_id, &options) => Some(r),
        _ = cancel.cancelled() => None,
    };
    let plan = match licence {
        Some((key, licence_id)) => {
            // Persist what this request means for the customer, then let the
            // dispatcher consider the next one.
            let settled = settle_licence(app, row, &key, licence_id, planned).await;
            app.queue.clear_licence_in_flight(&key, row.id);
            app.queue.wake();
            match settled? {
                Ok(plan) => plan,
                Err(outcome) => return Ok(outcome),
            }
        }
        None => match planned {
            None => return Ok(Outcome::Cancelled),
            Some(Ok(plan)) => plan,
            Some(Err(f)) => return plan_failure_outcome(app, row, f).await,
        },
    };
    history::record(
        app,
        NewEvent::new("grab", format!("Downloading {}", row.title))
            .account(&row.account_id)
            .book(&row.book_id, &row.title)
            .data(json!({ "parts": plan.parts.len(), "provider": provider.as_str() })),
    )
    .await;

    // --- Pipeline ---
    let started = sqlx::query(&format!("UPDATE queue SET state = 'running' WHERE id = ? AND state IN {ACTIVE_STATES_SQL}"))
        .bind(row.id)
        .execute(&app.db)
        .await?
        .rows_affected();
    if started == 0 {
        // Someone else moved the item on (it was removed with its account).
        return Ok(Outcome::Cancelled);
    }
    queue::publish_id(app, row.id).await;

    let values = books::template_values(&meta, &source);
    let tpl = if meta.series.is_some() {
        &settings.media.folder_template
    } else {
        &settings.media.no_series_folder_template
    };
    let rel = reserve_dest(app, row, &template::render(tpl, &values)).await?;
    let job = PipelineJob {
        plan,
        meta,
        format: match format {
            Format::M4b => OutputFormat::M4b,
            Format::Mp3 => OutputFormat::Mp3,
        },
        work_dir: app.cfg.work_dir().join(row.id.to_string()),
        dest_dir: app.cfg.library_dir.join(&rel),
        file_stem: template::file_stem(&values),
        write_sidecars: settings.media.write_sidecars,
    };

    let progress: ProgressFn = {
        let app = Arc::downgrade(app);
        let id = row.id;
        Arc::new(move |p: Progress| {
            let Some(app) = app.upgrade() else { return };
            if app.queue.set_progress(id, p) {
                // Publishing needs the row; do it off the pipeline's task.
                tokio::spawn(async move { queue::publish_id(&app, id).await });
            }
        })
    };

    let mut retried = false;
    loop {
        let err = match app.executor.run(job.clone(), progress.clone(), cancel.clone()).await {
            Ok(output) => {
                return Ok(Outcome::Done { output, rel_path: rel.to_string_lossy().into_owned(), format });
            }
            Err(e) => e,
        };
        if !retried && matches!(err.kind, FailureKind::Network | FailureKind::Http) {
            // A blip: the plan (and its licence) is still good, so try again now
            // rather than going back to the queue and asking for a new one.
            retried = true;
            tracing::warn!("{}: {}; retrying with the same plan", row.title, err.message);
            tokio::select! {
                _ = tokio::time::sleep(INLINE_RETRY_DELAY) => continue,
                _ = cancel.cancelled() => return Ok(Outcome::Cancelled),
            }
        }
        return Ok(pipeline_failure_outcome(err));
    }
}

/// How a pipeline failure is handled: `PipelineError::is_retryable` failures go back
/// to the queue for a fresh plan; local problems pause the queue.
fn pipeline_failure_outcome(e: PipelineError) -> Outcome {
    if e.kind == FailureKind::Cancelled {
        return Outcome::Cancelled;
    }
    if e.is_retryable() {
        return Outcome::Failed { message: e.message, retryable: true };
    }
    let hold = match e.kind {
        FailureKind::DiskFull => Some(HoldKind::DiskFull),
        FailureKind::Ffmpeg => Some(HoldKind::Ffmpeg),
        FailureKind::Io => Some(HoldKind::Io),
        _ => None,
    };
    match hold {
        Some(kind) => Outcome::Halt { kind, message: e.message },
        None => Outcome::Failed { message: e.message, retryable: false },
    }
}

/// Record a licence request's outcome and its consequences for the customer. Returns
/// the plan to run, or the outcome that ends this attempt.
async fn settle_licence(
    app: &AppState,
    row: &QueueRow,
    key: &str,
    licence_id: i64,
    planned: Option<Result<DownloadPlan, PlanFailure>>,
) -> anyhow::Result<Result<DownloadPlan, Outcome>> {
    match planned {
        None => {
            set_licence_outcome(app, licence_id, "cancelled").await;
            Ok(Err(Outcome::Cancelled))
        }
        Some(Ok(plan)) if plan.licence_throttled => {
            // Audible refused the AAXC licence as throttled and the plan came from the
            // AAX fallback: usable, but the customer is throttled all the same. This is
            // not a successful probe, so the cooldown level is not reset.
            set_licence_outcome(app, licence_id, "throttled").await;
            apply_throttle(app, row, key, "Audible throttled the licence request; downloading the AAX fallback").await?;
            clear_title_backoff(app, &row.book_id).await?;
            Ok(Ok(plan))
        }
        Some(Ok(plan)) => {
            set_licence_outcome(app, licence_id, "granted").await;
            licence_granted(app, row, key).await?;
            Ok(Ok(plan))
        }
        Some(Err(f)) => {
            let outcome = match f.kind {
                PlanFailureKind::Throttled => "throttled",
                PlanFailureKind::Denied => "denied",
                PlanFailureKind::AuthExpired | PlanFailureKind::Unavailable | PlanFailureKind::Other => "error",
            };
            set_licence_outcome(app, licence_id, outcome).await;
            let ended = match f.kind {
                PlanFailureKind::Throttled => {
                    apply_throttle(app, row, key, &f.message).await?;
                    Outcome::Throttled(f.message)
                }
                PlanFailureKind::Unavailable => {
                    let until = apply_unavailable_hold(app, key).await?;
                    Outcome::Unavailable { message: f.message, until }
                }
                _ => plan_failure_outcome(app, row, f).await?,
            };
            Ok(Err(ended))
        }
    }
}

/// The outcome of a plan failure that needs no customer-level pacing state.
async fn plan_failure_outcome(app: &App, row: &QueueRow, f: PlanFailure) -> anyhow::Result<Outcome> {
    Ok(match f.kind {
        PlanFailureKind::Throttled => Outcome::Throttled(f.message),
        PlanFailureKind::Denied => Outcome::Denied(f.message),
        PlanFailureKind::AuthExpired => {
            // Recorded now so the dispatcher holds the account's other items.
            accounts::set_auth_status(app, &row.account_id, "expired").await?;
            Outcome::AuthExpired(f.message)
        }
        PlanFailureKind::Unavailable | PlanFailureKind::Other => Outcome::Failed { message: f.message, retryable: true },
    })
}

async fn record_licence_request(app: &App, row: &QueueRow, key: &str) -> sqlx::Result<i64> {
    let settings = app.settings();
    let now = time::now();
    let spacing = pacing::pick_spacing(
        &mut rand::thread_rng(),
        settings.audible.spacing_min_minutes,
        settings.audible.spacing_max_minutes,
    );
    accounts::ensure_pacing(&app.db, key).await?;
    let mut tx = app.db.begin().await?;
    sqlx::query("UPDATE licence_pacing SET next_licence_at = ? WHERE pacing_key = ?")
        .bind(time::fmt(now + spacing))
        .bind(key)
        .execute(&mut *tx)
        .await?;
    let id = sqlx::query("INSERT INTO licence_requests (account_id, pacing_key, book_id, time) VALUES (?, ?, ?, ?)")
        .bind(&row.account_id)
        .bind(key)
        .bind(&row.book_id)
        .bind(time::fmt(now))
        .execute(&mut *tx)
        .await?
        .last_insert_rowid();
    // A `force` is spent by the request it allowed, whatever Audible answers: it must
    // not carry a retry of this item past a cooldown that answer starts.
    sqlx::query("UPDATE queue SET force = 0 WHERE id = ?").bind(row.id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(id)
}

async fn set_licence_outcome(app: &App, id: i64, outcome: &str) {
    if let Err(e) = sqlx::query("UPDATE licence_requests SET outcome = ? WHERE id = ?")
        .bind(outcome)
        .bind(id)
        .execute(&app.db)
        .await
    {
        tracing::warn!("Could not record a licence outcome: {e}");
    }
}

/// A granted licence ends any throttle episode (a successful probe resets the level)
/// and any backoff on the title.
async fn licence_granted(app: &App, row: &QueueRow, key: &str) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE licence_pacing SET cooldown_level = 0, cooldown_until = NULL, cooldown_reason = NULL,
                hold_until = NULL, hold_reason = NULL
         WHERE pacing_key = ?",
    )
    .bind(key)
    .execute(&app.db)
    .await?;
    clear_title_backoff(app, &row.book_id).await
}

async fn clear_title_backoff(app: &App, book_id: &str) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM title_backoff WHERE book_id = ?").bind(book_id).execute(&app.db).await?;
    Ok(())
}

/// Start (or extend) the customer's throttle cooldown, and tell the user.
async fn apply_throttle(app: &AppState, row: &QueueRow, key: &str, message: &str) -> anyhow::Result<()> {
    let now = time::now();
    let settings = app.settings();
    accounts::ensure_pacing(&app.db, key).await?;
    let level: i64 = sqlx::query_scalar("SELECT cooldown_level FROM licence_pacing WHERE pacing_key = ?")
        .bind(key)
        .fetch_one(&app.db)
        .await?;
    let (until, new_level) = pacing::after_throttle(settings.audible.throttle_cooldown_hours, level.max(0) as u32, now);
    let reason = format!("Audible is throttling licence requests (throttle #{new_level})");
    sqlx::query("UPDATE licence_pacing SET cooldown_until = ?, cooldown_level = ?, cooldown_reason = ? WHERE pacing_key = ?")
        .bind(time::fmt(until))
        .bind(new_level as i64)
        .bind(&reason)
        .bind(key)
        .execute(&app.db)
        .await?;
    history::record(
        app,
        NewEvent::new(
            "licenceThrottled",
            format!("Audible throttled this account; no licence requests until {}", time::local_label(until)),
        )
        .account(&row.account_id)
        .book(&row.book_id, &row.title)
        .data(json!({ "cooldownUntil": time::fmt(until), "level": new_level, "error": message })),
    )
    .await;
    publish_customer_accounts(app, key).await;
    Ok(())
}

/// Audible answered a licence request with a server or network error: stop the
/// customer's licence requests for a short while instead of trying title after title.
async fn apply_unavailable_hold(app: &App, key: &str) -> anyhow::Result<DateTime<Utc>> {
    let until = time::now() + pacing::UNAVAILABLE_HOLD;
    accounts::ensure_pacing(&app.db, key).await?;
    sqlx::query("UPDATE licence_pacing SET hold_until = ?, hold_reason = ? WHERE pacing_key = ?")
        .bind(time::fmt(until))
        .bind(format!("Audible unavailable, retrying at {}", time::local_label(until)))
        .bind(key)
        .execute(&app.db)
        .await?;
    Ok(until)
}

/// Publish every account that shares `key` (its cooldown changed).
async fn publish_customer_accounts(app: &App, key: &str) {
    let ids: Vec<String> = sqlx::query_scalar("SELECT account_id FROM account_state WHERE pacing_key = ?")
        .bind(key)
        .fetch_all(&app.db)
        .await
        .unwrap_or_default();
    for id in ids {
        accounts::publish(app, &id).await;
    }
}

/// Pick the item's library folder and record it on the row. A folder another book has
/// already been saved to, or that another book's queue item is using, gets the book id
/// appended (`Title [B0123]`), so two books never write into one folder.
async fn reserve_dest(app: &App, row: &QueueRow, rendered: &Path) -> sqlx::Result<PathBuf> {
    let _guard = app.queue.dest_lock.lock().await;
    let mut rel = rendered.to_path_buf();
    if dest_taken(app, row, &rel).await? {
        let alt = template::disambiguate(&rel, &row.book_id);
        tracing::info!(
            "{}: folder {} belongs to another book; using {}",
            row.title,
            rel.display(),
            alt.display()
        );
        if dest_taken(app, row, &alt).await? {
            tracing::warn!("{}: folder {} is also taken by another book", row.title, alt.display());
        }
        rel = alt;
    }
    sqlx::query("UPDATE queue SET dest_path = ? WHERE id = ?")
        .bind(rel.to_string_lossy().as_ref())
        .bind(row.id)
        .execute(&app.db)
        .await?;
    Ok(rel)
}

/// Is `rel` recorded or reserved for a different book? Compared case-insensitively,
/// since libraries often live on case-insensitive filesystems.
async fn dest_taken(app: &App, row: &QueueRow, rel: &Path) -> sqlx::Result<bool> {
    let path = rel.to_string_lossy().into_owned();
    let taken: Option<i64> = sqlx::query_scalar(&format!(
        "SELECT 1 FROM downloads WHERE lower(output_path) = lower(?1) AND book_id != ?2
         UNION ALL
         SELECT 1 FROM queue WHERE lower(dest_path) = lower(?1) AND book_id != ?2 AND state IN {SLOT_STATES_SQL}
         LIMIT 1"
    ))
    .bind(&path)
    .bind(&row.book_id)
    .fetch_optional(&app.db)
    .await?;
    Ok(taken.is_some())
}

/// The lower free space of the library and work folders, if it can be read.
async fn min_free_space(app: &App) -> Option<u64> {
    let dirs = [app.cfg.library_dir.clone(), app.cfg.work_dir()];
    tokio::task::spawn_blocking(move || dirs.iter().filter_map(|d| fs2::available_space(d).ok()).min())
        .await
        .ok()
        .flatten()
}

/// Apply an outcome to the queue row. Every transition is conditional on the item
/// still being this worker's (`planning`/`running`): if something else moved it on
/// (its account was removed), the worker leaves it alone.
pub async fn finish(app: &AppState, row: &QueueRow, outcome: Outcome, user_cancel: bool) -> anyhow::Result<()> {
    let now = time::now();
    match outcome {
        Outcome::Done { output, rel_path, format } => {
            let mut tx = app.db.begin().await?;
            sqlx::query(
                "INSERT INTO downloads (book_id, account_id, provider, output_path, format, total_bytes, audio_files, downloaded_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(book_id) DO UPDATE SET account_id = excluded.account_id, provider = excluded.provider,
                   output_path = excluded.output_path, format = excluded.format, total_bytes = excluded.total_bytes,
                   audio_files = excluded.audio_files, downloaded_at = excluded.downloaded_at",
            )
            .bind(&row.book_id)
            .bind(&row.account_id)
            .bind(&row.provider)
            .bind(&rel_path)
            .bind(format.as_str())
            .bind(output.total_bytes as i64)
            .bind(output.audio_files.len() as i64)
            .bind(time::fmt(now))
            .execute(&mut *tx)
            .await?;
            let moved = sqlx::query(&format!(
                "UPDATE queue SET state = 'completed', finished_at = ?, error = NULL, waiting_reason = NULL
                 WHERE id = ? AND state IN {ACTIVE_STATES_SQL}"
            ))
            .bind(time::fmt(now))
            .bind(row.id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            // The files are in the library either way, so the download is recorded.
            tx.commit().await?;
            if moved == 0 {
                tracing::warn!("{} finished after its queue item was removed", row.title);
            }
            queue::publish_id(app, row.id).await;
            history::record(
                app,
                NewEvent::new("download", format!("Imported {}", row.title))
                    .account(&row.account_id)
                    .book(&row.book_id, &row.title)
                    .data(json!({
                        "outputPath": rel_path,
                        "format": format.as_str(),
                        "bytes": output.total_bytes,
                        "files": output.audio_files.len(),
                        "chapters": output.chapter_count,
                    })),
            )
            .await;
            accounts::publish(app, &row.account_id).await;
        }
        Outcome::Throttled(message) => {
            // The cooldown is recorded already. Not this title's fault: back to the
            // queue, attempts untouched.
            requeue(app, row, None, Some(message)).await?;
            accounts::publish(app, &row.account_id).await;
        }
        Outcome::Unavailable { message, until } => {
            // The customer's hold is recorded already; this title also counts an
            // attempt, so one title Audible always fails on can't block the rest forever.
            let attempts = row.attempts + 1;
            match pacing::retry_delay(attempts as u32) {
                Some(d) => {
                    let at = (now + d).max(until);
                    let reason = format!("Audible unavailable, retrying at {}", time::local_label(at));
                    wait_for_retry(app, row, attempts, &message, at, &reason).await?;
                }
                None => fail(app, row, attempts, &message, now).await?,
            }
        }
        Outcome::Denied(message) => {
            let level: i64 = sqlx::query_scalar("SELECT level FROM title_backoff WHERE book_id = ?")
                .bind(&row.book_id)
                .fetch_optional(&app.db)
                .await?
                .unwrap_or(0);
            let until = now + pacing::title_backoff_duration(level.max(0) as u32);
            sqlx::query(
                "INSERT INTO title_backoff (book_id, account_id, level, until, reason) VALUES (?, ?, ?, ?, ?)
                 ON CONFLICT(book_id) DO UPDATE SET account_id = excluded.account_id, level = excluded.level,
                   until = excluded.until, reason = excluded.reason",
            )
            .bind(&row.book_id)
            .bind(&row.account_id)
            .bind(level + 1)
            .bind(time::fmt(until))
            .bind(&message)
            .execute(&app.db)
            .await?;
            // The title leaves the queue; back-fill picks it up again once the backoff
            // has passed, and a manual download may retry it at any time.
            sqlx::query(&format!("DELETE FROM queue WHERE id = ? AND state IN {ACTIVE_STATES_SQL}"))
                .bind(row.id)
                .execute(&app.db)
                .await?;
            app.hub.publish("queueRemoved", &json!({ "id": row.id.to_string() }));
            history::record(
                app,
                NewEvent::new("licenceDenied", format!("Audible refused a licence for {}: {message}", row.title))
                    .account(&row.account_id)
                    .book(&row.book_id, &row.title)
                    .data(json!({ "backoffUntil": time::fmt(until), "level": level + 1 })),
            )
            .await;
        }
        Outcome::AuthExpired(message) => {
            accounts::set_auth_status(app, &row.account_id, "expired").await?;
            requeue(app, row, None, Some(message.clone())).await?;
            history::record(
                app,
                NewEvent::new("authExpired", format!("Sign-in for this account is no longer accepted: {message}"))
                    .account(&row.account_id),
            )
            .await;
            accounts::publish(app, &row.account_id).await;
        }
        Outcome::Cancelled if user_cancel => {
            sqlx::query(&format!(
                "UPDATE queue SET state = 'cancelled', finished_at = ?, waiting_reason = NULL
                 WHERE id = ? AND state IN {ACTIVE_STATES_SQL}"
            ))
            .bind(time::fmt(now))
            .bind(row.id)
            .execute(&app.db)
            .await?;
            queue::publish_id(app, row.id).await;
        }
        Outcome::Cancelled => {
            // Shutdown: leave it for the next start.
            requeue(app, row, None, None).await?;
        }
        Outcome::Halt { kind, message } => {
            let free = if kind == HoldKind::DiskFull { min_free_space(app).await } else { None };
            app.queue.set_hold(kind, message.clone(), free);
            let hold = app.queue.hold();
            let summary = hold.as_ref().map(|h| h.waiting_reason()).unwrap_or_default();
            tracing::error!("{} failed: {message}. {summary}", row.title);
            fail(app, row, row.attempts + 1, &message, now).await?;
        }
        Outcome::Failed { message, retryable } => {
            let attempts = row.attempts + 1;
            let delay = if retryable { pacing::retry_delay(attempts as u32) } else { None };
            match delay {
                Some(d) => {
                    let at = now + d;
                    let reason = format!("Attempt {attempts} failed; retrying at {}", time::local_label(at));
                    tracing::warn!("{} failed (attempt {attempts}): {message}", row.title);
                    wait_for_retry(app, row, attempts, &message, at, &reason).await?;
                }
                None => fail(app, row, attempts, &message, now).await?,
            }
        }
    }
    Ok(())
}

async fn wait_for_retry(
    app: &AppState,
    row: &QueueRow,
    attempts: i64,
    message: &str,
    at: DateTime<Utc>,
    reason: &str,
) -> sqlx::Result<()> {
    sqlx::query(&format!(
        "UPDATE queue SET state = 'waiting', attempts = ?, error = ?, not_before = ?, waiting_reason = ?
         WHERE id = ? AND state IN {ACTIVE_STATES_SQL}"
    ))
    .bind(attempts)
    .bind(message)
    .bind(time::fmt(at))
    .bind(reason)
    .bind(row.id)
    .execute(&app.db)
    .await?;
    queue::publish_id(app, row.id).await;
    Ok(())
}

async fn fail(app: &AppState, row: &QueueRow, attempts: i64, message: &str, now: DateTime<Utc>) -> sqlx::Result<()> {
    let moved = sqlx::query(&format!(
        "UPDATE queue SET state = 'failed', attempts = ?, error = ?, finished_at = ?, waiting_reason = NULL
         WHERE id = ? AND state IN {ACTIVE_STATES_SQL}"
    ))
    .bind(attempts)
    .bind(message)
    .bind(time::fmt(now))
    .bind(row.id)
    .execute(&app.db)
    .await?
    .rows_affected();
    queue::publish_id(app, row.id).await;
    if moved == 1 {
        history::record(
            app,
            NewEvent::new("downloadFailed", format!("{} failed: {message}", row.title))
                .account(&row.account_id)
                .book(&row.book_id, &row.title)
                .data(json!({ "attempts": attempts })),
        )
        .await;
    }
    Ok(())
}

/// Back to `queued` without counting an attempt.
async fn requeue(app: &AppState, row: &QueueRow, reason: Option<String>, error: Option<String>) -> sqlx::Result<()> {
    sqlx::query(&format!(
        "UPDATE queue SET state = 'queued', waiting_reason = ?, error = ?, started_at = NULL
         WHERE id = ? AND state IN {ACTIVE_STATES_SQL}"
    ))
    .bind(reason)
    .bind(error)
    .bind(row.id)
    .execute(&app.db)
    .await?;
    queue::publish_id(app, row.id).await;
    Ok(())
}
