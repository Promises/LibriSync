//! One queue item, start to finish: plan (for Audible, the licence request) →
//! pipeline → import, and the state transition that follows from the outcome.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rust_core::pipeline::{FailureKind, OutputFormat, PipelineJob, PipelineOutput, Progress, ProgressFn};
use rust_core::providers::ProviderId;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::accounts;
use crate::books;
use crate::executor::PlanFailureKind;
use crate::history::{self, NewEvent};
use crate::pacing;
use crate::queue::{self, QueueRow};
use crate::settings::Format;
use crate::state::{App, AppState};
use crate::template;
use crate::time;

/// How an attempt ended.
#[derive(Debug)]
pub enum Outcome {
    Done { output: PipelineOutput, rel_path: String, format: Format },
    Throttled(String),
    Denied(String),
    AuthExpired(String),
    Failed { message: String, retryable: bool },
    Cancelled,
}

pub async fn run(app: AppState, id: i64, cancel: CancellationToken, user_cancelled: Arc<AtomicBool>) {
    let row = match queue::get(&app.db, id).await {
        Ok(Some(row)) => row,
        Ok(None) => {
            app.queue.unregister(id);
            return;
        }
        Err(e) => {
            tracing::error!("Queue item {id} vanished: {e}");
            app.queue.unregister(id);
            return;
        }
    };

    // The attempt runs in its own task so that a panic anywhere below (a provider, the
    // pipeline) fails this item instead of leaving it "planning" forever.
    let attempt = {
        let app = app.clone();
        let row = row.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move { execute(&app, &row, &cancel).await })
    };
    let outcome = match attempt.await {
        Ok(o) => o,
        Err(e) => Outcome::Failed { message: format!("Internal error: {e}"), retryable: true },
    };

    if row.provider == "audible" {
        app.queue.set_licence_in_flight(&row.account_id, false);
    }
    let user_cancel = user_cancelled.load(Ordering::SeqCst);
    if let Err(e) = finish(&app, &row, outcome, user_cancel).await {
        tracing::error!("Could not record the result of queue item {id}: {e:#}");
    }
    app.queue.unregister(id);
    app.queue.wake();
}

async fn execute(app: &AppState, row: &QueueRow, cancel: &CancellationToken) -> Outcome {
    match execute_inner(app, row, cancel).await {
        Ok(o) => o,
        Err(e) => Outcome::Failed { message: format!("{e:#}"), retryable: true },
    }
}

async fn execute_inner(app: &AppState, row: &QueueRow, cancel: &CancellationToken) -> anyhow::Result<Outcome> {
    let settings = app.settings();
    let Some(account) = accounts::load_json(app, &row.account_id).await? else {
        return Ok(Outcome::Failed { message: "The account that owns this book has been removed".into(), retryable: false });
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
    let licence_id = if provider == ProviderId::Audible {
        Some(record_licence_request(app, row).await?)
    } else {
        None
    };
    let planned = tokio::select! {
        r = app.executor.plan(&app.core, provider, &account, &row.book_id, &options) => Some(r),
        _ = cancel.cancelled() => None,
    };
    if provider == ProviderId::Audible {
        app.queue.set_licence_in_flight(&row.account_id, false);
        app.queue.wake();
    }
    let plan = match planned {
        None => {
            set_licence_outcome(app, licence_id, "cancelled").await;
            return Ok(Outcome::Cancelled);
        }
        Some(Err(f)) => {
            let outcome = match f.kind {
                PlanFailureKind::Throttled => ("throttled", Outcome::Throttled(f.message)),
                PlanFailureKind::Denied => ("denied", Outcome::Denied(f.message)),
                PlanFailureKind::AuthExpired => ("error", Outcome::AuthExpired(f.message)),
                PlanFailureKind::Other => ("error", Outcome::Failed { message: f.message, retryable: true }),
            };
            set_licence_outcome(app, licence_id, outcome.0).await;
            return Ok(outcome.1);
        }
        Some(Ok(plan)) => plan,
    };
    if provider == ProviderId::Audible {
        set_licence_outcome(app, licence_id, "granted").await;
        licence_granted(app, row).await?;
    }
    history::record(
        app,
        NewEvent::new("grab", format!("Downloading {}", row.title))
            .account(&row.account_id)
            .book(&row.book_id, &row.title)
            .data(json!({ "parts": plan.parts.len(), "provider": provider.as_str() })),
    )
    .await;

    // --- Pipeline ---
    sqlx::query("UPDATE queue SET state = 'running' WHERE id = ?")
        .bind(row.id)
        .execute(&app.db)
        .await?;
    queue::publish_id(app, row.id).await;

    let values = books::template_values(&meta, &source);
    let tpl = if meta.series.is_some() {
        &settings.media.folder_template
    } else {
        &settings.media.no_series_folder_template
    };
    let rel = template::render(tpl, &values);
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

    match app.executor.run(job, progress, cancel.clone()).await {
        Ok(output) => Ok(Outcome::Done { output, rel_path: rel.to_string_lossy().into_owned(), format }),
        Err(e) if e.kind == FailureKind::Cancelled => Ok(Outcome::Cancelled),
        Err(e) => Ok(Outcome::Failed {
            retryable: !matches!(e.kind, FailureKind::InvalidPlan),
            message: e.message,
        }),
    }
}

async fn record_licence_request(app: &App, row: &QueueRow) -> sqlx::Result<i64> {
    let settings = app.settings();
    let now = time::now();
    let spacing = pacing::pick_spacing(
        &mut rand::thread_rng(),
        settings.audible.spacing_min_minutes,
        settings.audible.spacing_max_minutes,
    );
    accounts::ensure_state(app, &row.account_id).await?;
    sqlx::query("UPDATE account_state SET next_licence_at = ? WHERE account_id = ?")
        .bind(time::fmt(now + spacing))
        .bind(&row.account_id)
        .execute(&app.db)
        .await?;
    let id = sqlx::query("INSERT INTO licence_requests (account_id, book_id, time) VALUES (?, ?, ?)")
        .bind(&row.account_id)
        .bind(&row.book_id)
        .bind(time::fmt(now))
        .execute(&app.db)
        .await?
        .last_insert_rowid();
    Ok(id)
}

async fn set_licence_outcome(app: &App, id: Option<i64>, outcome: &str) {
    let Some(id) = id else { return };
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
async fn licence_granted(app: &App, row: &QueueRow) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE account_state SET cooldown_level = 0, cooldown_until = NULL, cooldown_reason = NULL WHERE account_id = ?",
    )
    .bind(&row.account_id)
    .execute(&app.db)
    .await?;
    sqlx::query("DELETE FROM title_backoff WHERE book_id = ?")
        .bind(&row.book_id)
        .execute(&app.db)
        .await?;
    Ok(())
}

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
            sqlx::query("UPDATE queue SET state = 'completed', finished_at = ?, error = NULL, waiting_reason = NULL WHERE id = ?")
                .bind(time::fmt(now))
                .bind(row.id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
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
            let settings = app.settings();
            let level: i64 = sqlx::query_scalar("SELECT cooldown_level FROM account_state WHERE account_id = ?")
                .bind(&row.account_id)
                .fetch_optional(&app.db)
                .await?
                .unwrap_or(0);
            let (until, new_level) =
                pacing::after_throttle(settings.audible.throttle_cooldown_hours, level.max(0) as u32, now);
            let reason = format!("Audible is throttling licence requests (throttle #{new_level})");
            sqlx::query(
                "UPDATE account_state SET cooldown_until = ?, cooldown_level = ?, cooldown_reason = ? WHERE account_id = ?",
            )
            .bind(time::fmt(until))
            .bind(new_level as i64)
            .bind(&reason)
            .bind(&row.account_id)
            .execute(&app.db)
            .await?;
            // Not this title's fault: back to the queue, attempts untouched.
            requeue(app, row, None, Some(message.clone())).await?;
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
            accounts::publish(app, &row.account_id).await;
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
            sqlx::query("DELETE FROM queue WHERE id = ?").bind(row.id).execute(&app.db).await?;
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
            sqlx::query("UPDATE queue SET state = 'cancelled', finished_at = ?, waiting_reason = NULL WHERE id = ?")
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
        Outcome::Failed { message, retryable } => {
            let attempts = row.attempts + 1;
            let delay = if retryable { pacing::retry_delay(attempts as u32) } else { None };
            match delay {
                Some(d) => {
                    let at = now + d;
                    let reason = format!("Attempt {attempts} failed; retrying at {}", time::local_label(at));
                    sqlx::query(
                        "UPDATE queue SET state = 'waiting', attempts = ?, error = ?, not_before = ?, waiting_reason = ? WHERE id = ?",
                    )
                    .bind(attempts)
                    .bind(&message)
                    .bind(time::fmt(at))
                    .bind(&reason)
                    .bind(row.id)
                    .execute(&app.db)
                    .await?;
                    tracing::warn!("{} failed (attempt {attempts}): {message}", row.title);
                    queue::publish_id(app, row.id).await;
                }
                None => {
                    sqlx::query(
                        "UPDATE queue SET state = 'failed', attempts = ?, error = ?, finished_at = ?, waiting_reason = NULL WHERE id = ?",
                    )
                    .bind(attempts)
                    .bind(&message)
                    .bind(time::fmt(now))
                    .bind(row.id)
                    .execute(&app.db)
                    .await?;
                    queue::publish_id(app, row.id).await;
                    history::record(
                        app,
                        NewEvent::new("downloadFailed", format!("{} failed: {message}", row.title))
                            .account(&row.account_id)
                            .book(&row.book_id, &row.title)
                            .data(json!({ "attempts": attempts })),
                    )
                    .await;
                }
            }
        }
    }
    Ok(())
}

/// Back to `queued` without counting an attempt.
async fn requeue(app: &AppState, row: &QueueRow, reason: Option<String>, error: Option<String>) -> sqlx::Result<()> {
    sqlx::query("UPDATE queue SET state = 'queued', waiting_reason = ?, error = ?, started_at = NULL WHERE id = ?")
        .bind(reason)
        .bind(error)
        .bind(row.id)
        .execute(&app.db)
        .await?;
    queue::publish_id(app, row.id).await;
    Ok(())
}
