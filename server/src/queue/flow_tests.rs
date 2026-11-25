//! Queue state transitions end to end, with a scripted executor in place of the
//! provider plan and the pipeline.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use rust_core::pipeline::{FailureKind, PipelineError};
use rust_core::storage::queries::{self, StoreBook, StoreSeries};

use super::dispatch::dispatch_once;
use super::*;
use crate::executor::{PlanFailure, PlanFailureKind};
use crate::state::AppState;
use crate::test_support::{test_app_with, FakeExecutor};

async fn add_account(app: &AppState, id: &str, provider: &str) {
    let json = serde_json::json!({
        "account_id": id,
        "account_name": id,
        "provider": provider,
        "locale": { "country_code": "us" },
        "identity": { "access_token": "test-token" },
    });
    rust_core::storage::accounts::save_account(app.core.pool(), id, &json.to_string())
        .await
        .unwrap();
    crate::accounts::ensure_state(app, id).await.unwrap();
}

async fn add_book(app: &AppState, account: &str, provider: &str, id: &str, title: &str) {
    let authors = vec!["Ann Author".to_string()];
    queries::insert_store_book(
        app.core.pool(),
        &StoreBook {
            source: provider,
            product_id: id,
            title,
            subtitle: None,
            authors: &authors,
            narrators: &[],
            publisher: None,
            description: "",
            length_in_minutes: 60,
            cover_url: None,
            language: "english",
            date_published: Some("2020-01-01"),
            is_abridged: false,
            is_downloadable: true,
            series: Some(StoreSeries { series_key: "s1", name: "Saga", order: Some("2") }),
            account,
        },
    )
    .await
    .unwrap();
}

/// Dispatch, then wait until every worker has finished.
async fn settle(app: &AppState) {
    dispatch_once(app).await.unwrap();
    for _ in 0..200 {
        if app.queue.running_count() == 0 {
            return;
        }
        tokio::time::sleep(StdDuration::from_millis(10)).await;
    }
    panic!("workers did not finish");
}

async fn row(app: &AppState, book: &str) -> Option<QueueRow> {
    sqlx::query_as::<_, QueueRow>(&format!(
        "SELECT {COLUMNS} FROM queue WHERE book_id = ? ORDER BY id DESC LIMIT 1"
    ))
    .bind(book)
    .fetch_optional(&app.db)
    .await
    .unwrap()
}

async fn status(app: &AppState, book: &str) -> String {
    crate::books::get(&app.db, book).await.unwrap().unwrap().status
}

#[tokio::test]
async fn priorities_order_the_queue_and_requeueing_upgrades() {
    let (app, _dir) = test_app_with(Arc::new(FakeExecutor::default())).await;
    add_account(&app, "chirp:1", "chirp").await;
    for b in ["a", "b", "c"] {
        add_book(&app, "chirp:1", "chirp", b, b).await;
    }
    enqueue(&app, "a", Priority::Backfill, false).await.unwrap();
    enqueue(&app, "b", Priority::New, false).await.unwrap();
    enqueue(&app, "c", Priority::Manual, false).await.unwrap();
    let order: Vec<String> = list_pending(&app.db).await.unwrap().into_iter().map(|r| r.book_id).collect();
    assert_eq!(order, vec!["c", "b", "a"]);

    // Asking again for "a" manually upgrades it instead of duplicating it.
    let again = enqueue(&app, "a", Priority::Manual, false).await.unwrap();
    assert_eq!(again.priority(), Priority::Manual);
    assert_eq!(list_pending(&app.db).await.unwrap().len(), 3);
    let order: Vec<String> = list_pending(&app.db).await.unwrap().into_iter().map(|r| r.book_id).collect();
    assert_eq!(order, vec!["a", "c", "b"], "same priority: oldest first");
    assert_eq!(status(&app, "a").await, "queued");
}

#[tokio::test]
async fn a_successful_download_is_recorded_under_the_template_path() {
    let (app, _dir) = test_app_with(Arc::new(FakeExecutor::default())).await;
    add_account(&app, "chirp:1", "chirp").await;
    add_book(&app, "chirp:1", "chirp", "chirp_1", "The Book").await;
    enqueue(&app, "chirp_1", Priority::Manual, false).await.unwrap();
    settle(&app).await;

    let r = row(&app, "chirp_1").await.unwrap();
    assert_eq!(r.state(), QueueState::Completed);
    let book = crate::books::get(&app.db, "chirp_1").await.unwrap().unwrap();
    assert_eq!(book.status, "downloaded");
    assert_eq!(book.output_path.as_deref(), Some("Ann Author/Saga/2 - The Book"));
    assert_eq!(book.format.as_deref(), Some("m4b"));

    // Downloaded books aren't re-queued automatically, only by hand.
    assert!(matches!(
        enqueue(&app, "chirp_1", Priority::Backfill, false).await,
        Err(EnqueueError::AlreadyDownloaded)
    ));
    assert!(enqueue(&app, "chirp_1", Priority::Manual, false).await.is_ok());
}

#[tokio::test]
async fn ordinary_failures_retry_with_backoff_then_fail() {
    let exec = Arc::new(FakeExecutor::default());
    // Each attempt runs the pipeline twice: once, then once more with the same plan.
    for _ in 0..8 {
        exec.runs.lock().unwrap().push_back(Err(PipelineError::new(FailureKind::Network, "reset")));
    }
    let (app, _dir) = test_app_with(exec.clone()).await;
    add_account(&app, "chirp:1", "chirp").await;
    add_book(&app, "chirp:1", "chirp", "x", "X").await;
    enqueue(&app, "x", Priority::New, false).await.unwrap();

    settle(&app).await;
    let r = row(&app, "x").await.unwrap();
    assert_eq!((r.state(), r.attempts), (QueueState::Waiting, 1));
    assert!(r.not_before().unwrap() > crate::time::now() + chrono::Duration::minutes(4));
    assert!(r.waiting_reason.as_deref().unwrap().starts_with("Attempt 1 failed"));

    // Still waiting: the dispatcher must not start it early.
    settle(&app).await;
    assert_eq!(row(&app, "x").await.unwrap().attempts, 1);

    for expected in 2..=4 {
        sqlx::query("UPDATE queue SET not_before = NULL WHERE book_id = 'x'").execute(&app.db).await.unwrap();
        settle(&app).await;
        assert_eq!(row(&app, "x").await.unwrap().attempts, expected);
    }
    let r = row(&app, "x").await.unwrap();
    assert_eq!(r.state(), QueueState::Failed);
    assert_eq!(r.error.as_deref(), Some("reset"));
    assert_eq!(exec.plan_calls.lock().unwrap().len(), 4, "one plan per attempt");
    assert_eq!(exec.run_calls.lock().unwrap().len(), 8, "each plan was run twice");
    assert_eq!(status(&app, "x").await, "failed");

    // Retrying (via a manual enqueue) resets the slot.
    let again = enqueue(&app, "x", Priority::Manual, false).await.unwrap();
    assert_eq!((again.state(), again.attempts), (QueueState::Queued, 0));
}

#[tokio::test]
async fn a_throttle_cools_the_account_down_and_holds_its_items() {
    let exec = Arc::new(FakeExecutor::default());
    exec.plans.lock().unwrap().push_back(Err(PlanFailure {
        kind: PlanFailureKind::Throttled,
        message: "CustomerThrottled".into(),
    }));
    let (app, _dir) = test_app_with(exec.clone()).await;
    add_account(&app, "amzn:us", "audible").await;
    add_book(&app, "amzn:us", "audible", "B1", "One").await;
    add_book(&app, "amzn:us", "audible", "B2", "Two").await;
    enqueue(&app, "B1", Priority::New, false).await.unwrap();
    enqueue(&app, "B2", Priority::New, false).await.unwrap();

    settle(&app).await;
    // Only one licence request went out (one in flight per account), and it was throttled.
    assert_eq!(exec.plan_calls.lock().unwrap().as_slice(), ["B1"]);
    let view = crate::accounts::get(&app, "amzn:us").await.unwrap().unwrap();
    let until = crate::time::parse(view.cooldown_until.as_deref().unwrap()).unwrap();
    let hours = (until - crate::time::now()).num_minutes() as f64 / 60.0;
    assert!((23.9..=24.0).contains(&hours), "{hours}");
    assert_eq!(view.licences_last24h, 1);
    let r = row(&app, "B1").await.unwrap();
    assert_eq!(r.attempts, 0, "a throttle is not the title's fault");

    settle(&app).await;
    for b in ["B1", "B2"] {
        let r = row(&app, b).await.unwrap();
        assert_eq!(r.state(), QueueState::Waiting);
        assert!(r.waiting_reason.as_deref().unwrap().starts_with("Account cooldown until"));
    }
    assert_eq!(exec.plan_calls.lock().unwrap().len(), 1, "no licence requests during the cooldown");

    // A manual download is refused unless forced.
    assert!(matches!(enqueue(&app, "B2", Priority::Manual, false).await, Err(EnqueueError::Cooldown(_))));
    enqueue(&app, "B2", Priority::Manual, true).await.unwrap();
    settle(&app).await;
    assert_eq!(exec.plan_calls.lock().unwrap().as_slice(), ["B1", "B2"]);
    assert_eq!(row(&app, "B2").await.unwrap().state(), QueueState::Completed);

    // That success was the probe: the throttle episode is over.
    let level: i64 = sqlx::query_scalar("SELECT cooldown_level FROM licence_pacing WHERE pacing_key = 'amzn'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(level, 0);
    assert!(crate::accounts::get(&app, "amzn:us").await.unwrap().unwrap().cooldown_until.is_none());
}

#[tokio::test]
async fn a_denial_backs_the_title_off_and_takes_it_out_of_the_queue() {
    let exec = Arc::new(FakeExecutor::default());
    exec.plans.lock().unwrap().push_back(Err(PlanFailure {
        kind: PlanFailureKind::Denied,
        message: "Ownership".into(),
    }));
    let (app, _dir) = test_app_with(exec).await;
    add_account(&app, "amzn:us", "audible").await;
    add_book(&app, "amzn:us", "audible", "B1", "One").await;
    enqueue(&app, "B1", Priority::Backfill, false).await.unwrap();
    settle(&app).await;

    assert!(row(&app, "B1").await.is_none());
    let book = crate::books::get(&app.db, "B1").await.unwrap().unwrap();
    assert_eq!(book.status, "cooldown");
    let until = crate::time::parse(book.backoff_until.as_deref().unwrap()).unwrap();
    assert!((until - crate::time::now()).num_hours() >= 23);

    // Back-fill and new respect the backoff; a manual download bypasses it.
    assert!(matches!(enqueue(&app, "B1", Priority::Backfill, false).await, Err(EnqueueError::Backoff(_))));
    assert!(enqueue(&app, "B1", Priority::Manual, false).await.is_ok());
    let wanted = crate::backfill::wanted_for_account(
        &app.db,
        "amzn:us",
        "audible",
        crate::settings::BackfillOrder::NewestFirst,
        10,
    )
    .await
    .unwrap();
    assert!(wanted.is_empty());
}

#[tokio::test]
async fn the_daily_cap_holds_even_manual_downloads() {
    let (app, _dir) = test_app_with(Arc::new(FakeExecutor::default())).await;
    let mut s = app.settings();
    s.audible.daily_licence_cap = 1;
    app.set_settings(s);
    add_account(&app, "amzn:us", "audible").await;
    add_book(&app, "amzn:us", "audible", "B1", "One").await;
    sqlx::query("INSERT INTO licence_requests (account_id, pacing_key, book_id, time, outcome) VALUES ('amzn:us', 'amzn', 'B0', ?, 'granted')")
        .bind(crate::time::now_str())
        .execute(&app.db)
        .await
        .unwrap();
    enqueue(&app, "B1", Priority::Manual, false).await.unwrap();
    settle(&app).await;
    let r = row(&app, "B1").await.unwrap();
    assert_eq!(r.state(), QueueState::Waiting);
    assert!(r.waiting_reason.as_deref().unwrap().starts_with("Licence budget: next slot"));
}

#[tokio::test]
async fn backfill_queues_one_audible_title_at_a_time_after_the_baseline() {
    let (app, _dir) = test_app_with(Arc::new(FakeExecutor::default())).await;
    add_account(&app, "amzn:us", "audible").await;
    add_account(&app, "chirp:1", "chirp").await;
    for b in ["B1", "B2", "B3"] {
        add_book(&app, "amzn:us", "audible", b, b).await;
    }
    for b in ["c1", "c2", "c3", "c4"] {
        add_book(&app, "chirp:1", "chirp", b, b).await;
    }
    crate::backfill::run_once(&app).await;
    assert!(list_pending(&app.db).await.unwrap().is_empty(), "no baseline yet");

    sqlx::query("UPDATE account_state SET baseline_at = ?").bind(crate::time::now_str()).execute(&app.db).await.unwrap();
    crate::backfill::run_once(&app).await;
    crate::backfill::run_once(&app).await;
    let pending = list_pending(&app.db).await.unwrap();
    let audible = pending.iter().filter(|r| r.provider == "audible").count();
    let chirp = pending.iter().filter(|r| r.provider == "chirp").count();
    assert_eq!((audible, chirp), (1, 3));
    assert!(pending.iter().all(|r| r.priority() == Priority::Backfill));

    // The one Chirp book left is wanted until it is skipped.
    let wanted = || async {
        crate::backfill::wanted_for_account(&app.db, "chirp:1", "chirp", crate::settings::BackfillOrder::NewestFirst, 10)
            .await
            .unwrap()
    };
    let left = wanted().await;
    assert_eq!(left.len(), 1);
    sqlx::query("INSERT INTO skips (book_id, created_at) VALUES (?, ?)")
        .bind(&left[0])
        .bind(crate::time::now_str())
        .execute(&app.db)
        .await
        .unwrap();
    assert!(wanted().await.is_empty());
}

#[tokio::test]
async fn startup_recovery_requeues_interrupted_items() {
    let (app, _dir) = test_app_with(Arc::new(FakeExecutor::default())).await;
    add_account(&app, "chirp:1", "chirp").await;
    add_book(&app, "chirp:1", "chirp", "a", "A").await;
    let r = enqueue(&app, "a", Priority::New, false).await.unwrap();
    sqlx::query("UPDATE queue SET state = 'running' WHERE id = ?").bind(r.id).execute(&app.db).await.unwrap();
    assert_eq!(super::dispatch::recover(&app.db).await.unwrap(), 1);
    assert_eq!(get(&app.db, r.id).await.unwrap().unwrap().state(), QueueState::Queued);
}

fn throttled() -> PlanFailure {
    PlanFailure { kind: PlanFailureKind::Throttled, message: "CustomerThrottled".into() }
}

async fn set_concurrency(app: &AppState, n: u32) {
    let mut s = app.settings();
    s.downloads.concurrency = n;
    app.set_settings(s);
}

async fn pacing_row(app: &AppState, key: &str) -> (Option<String>, i64, Option<String>) {
    sqlx::query_as("SELECT cooldown_until, cooldown_level, hold_until FROM licence_pacing WHERE pacing_key = ?")
        .bind(key)
        .fetch_optional(&app.db)
        .await
        .unwrap()
        .unwrap_or((None, 0, None))
}

async fn licence_outcomes(app: &AppState) -> Vec<String> {
    sqlx::query_scalar("SELECT outcome FROM licence_requests ORDER BY id").fetch_all(&app.db).await.unwrap()
}

async fn events(app: &AppState, event: &str) -> Vec<crate::history::HistoryItem> {
    crate::history::list(&app.db, &[event.to_string()], None, None, 1, 100).await.unwrap().1
}

/// The dispatcher loop runs in the background (as in production) with two workers;
/// the first licence request is throttled. No second request may start before the
/// cooldown is visible — repeated, since the old ordering lost this race only
/// sometimes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_throttle_is_recorded_before_the_next_licence_request_can_start() {
    for _round in 0..10 {
        let exec = Arc::new(FakeExecutor::default());
        exec.plans.lock().unwrap().push_back(Err(throttled()));
        let (app, _dir) = test_app_with(exec.clone()).await;
        set_concurrency(&app, 2).await;
        add_account(&app, "amzn:us", "audible").await;
        for b in ["B1", "B2", "B3"] {
            add_book(&app, "amzn:us", "audible", b, b).await;
            enqueue(&app, b, Priority::New, false).await.unwrap();
        }
        let dispatcher = tokio::spawn(super::dispatch::run(app.clone()));
        app.queue.wake();

        // Wait until the throttled item is back in the queue and nothing runs.
        let mut settled = false;
        for _ in 0..300 {
            tokio::time::sleep(StdDuration::from_millis(10)).await;
            let throttled_back = row(&app, "B1").await.unwrap().state() != QueueState::Planning;
            if throttled_back && app.queue.running_count() == 0 && !exec.plan_calls.lock().unwrap().is_empty() {
                settled = true;
                break;
            }
        }
        // Give a racing dispatch pass time to show itself.
        tokio::time::sleep(StdDuration::from_millis(50)).await;
        app.shutdown.cancel();
        dispatcher.await.unwrap();
        assert!(settled, "the queue never settled");
        assert_eq!(exec.plan_calls.lock().unwrap().as_slice(), ["B1"], "a licence request started into the throttle");
        assert!(pacing_row(&app, "amzn").await.0.is_some());
    }
}

#[tokio::test]
async fn a_throttled_licence_served_by_the_aax_fallback_downloads_but_cools_the_customer_down() {
    let exec = Arc::new(FakeExecutor::default());
    let mut plan = crate::test_support::empty_plan();
    plan.licence_throttled = true;
    exec.plans.lock().unwrap().push_back(Ok(plan));
    let (app, _dir) = test_app_with(exec.clone()).await;
    add_account(&app, "amzn:us", "audible").await;
    add_book(&app, "amzn:us", "audible", "B1", "One").await;
    add_book(&app, "amzn:us", "audible", "B2", "Two").await;
    // An earlier throttle episode whose cooldown has passed: this request is a probe.
    sqlx::query("INSERT INTO licence_pacing (pacing_key, cooldown_until, cooldown_level) VALUES ('amzn', ?, 1)")
        .bind(crate::time::fmt(crate::time::now() - chrono::Duration::hours(1)))
        .execute(&app.db)
        .await
        .unwrap();
    enqueue(&app, "B1", Priority::New, false).await.unwrap();
    enqueue(&app, "B2", Priority::New, false).await.unwrap();
    settle(&app).await;

    // The book downloaded...
    assert_eq!(row(&app, "B1").await.unwrap().state(), QueueState::Completed);
    assert_eq!(status(&app, "B1").await, "downloaded");
    // ...but the request counts as throttled, the probe failed (level 1 → 2, 48 h) and
    // the rest of the account waits.
    assert_eq!(licence_outcomes(&app).await, ["throttled"]);
    let (until, level, _) = pacing_row(&app, "amzn").await;
    assert_eq!(level, 2);
    let hours = (crate::time::parse(&until.unwrap()).unwrap() - crate::time::now()).num_minutes() as f64 / 60.0;
    assert!((47.9..=48.0).contains(&hours), "{hours}");
    assert_eq!(events(&app, "licenceThrottled").await.len(), 1);
    settle(&app).await;
    let r = row(&app, "B2").await.unwrap();
    assert_eq!(r.state(), QueueState::Waiting);
    assert!(r.waiting_reason.as_deref().unwrap().starts_with("Account cooldown until"));
    assert_eq!(exec.plan_calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn two_marketplaces_of_one_customer_share_pacing_and_removal_keeps_it() {
    let exec = Arc::new(FakeExecutor::default());
    exec.plans.lock().unwrap().push_back(Err(throttled()));
    let (app, _dir) = test_app_with(exec.clone()).await;
    let (us, uk) = ("amzn1.account.X:us", "amzn1.account.X:uk");
    add_account(&app, us, "audible").await;
    add_account(&app, uk, "audible").await;
    add_book(&app, us, "audible", "BU", "American").await;
    add_book(&app, uk, "audible", "BK", "British").await;
    enqueue(&app, "BU", Priority::New, false).await.unwrap();
    enqueue(&app, "BK", Priority::New, false).await.unwrap();
    settle(&app).await;
    settle(&app).await;

    assert_eq!(exec.plan_calls.lock().unwrap().as_slice(), ["BU"]);
    let r = row(&app, "BK").await.unwrap();
    assert!(r.waiting_reason.as_deref().unwrap().starts_with("Account cooldown until"), "{:?}", r.waiting_reason);
    let view = crate::accounts::get(&app, uk).await.unwrap().unwrap();
    assert!(view.cooldown_until.is_some(), "the other marketplace shows the cooldown");
    assert_eq!(view.licences_last24h, 1, "and counts the request against the shared cap");

    // Removing the account and adding it again doesn't forget the cooldown.
    let req = axum::http::Request::delete(format!("/api/v1/accounts/{}", urlencoding::encode(uk)))
        .header("X-Api-Key", app.api_key())
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = tower::ServiceExt::oneshot(crate::api::router(app.clone()), req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::NO_CONTENT);
    assert!(crate::accounts::get(&app, uk).await.unwrap().is_none());
    add_account(&app, uk, "audible").await;
    let view = crate::accounts::get(&app, uk).await.unwrap().unwrap();
    assert!(view.cooldown_until.is_some(), "re-adding the account keeps the cooldown");
    assert_eq!(view.licences_last24h, 1);
}

#[tokio::test]
async fn a_disk_full_failure_pauses_the_queue_without_spending_more_licences() {
    let exec = Arc::new(FakeExecutor::default());
    exec.runs.lock().unwrap().push_back(Err(PipelineError::new(FailureKind::DiskFull, "need 2 GB, 1 GB free")));
    let (app, _dir) = test_app_with(exec.clone()).await;
    add_account(&app, "amzn:us", "audible").await;
    add_book(&app, "amzn:us", "audible", "B1", "One").await;
    add_book(&app, "amzn:us", "audible", "B2", "Two").await;
    enqueue(&app, "B1", Priority::New, false).await.unwrap();
    enqueue(&app, "B2", Priority::New, false).await.unwrap();
    settle(&app).await;

    // Failed at once, not retried: retrying would re-plan and spend another licence.
    let r = row(&app, "B1").await.unwrap();
    assert_eq!((r.state(), r.attempts), (QueueState::Failed, 1));
    assert_eq!(r.error.as_deref(), Some("need 2 GB, 1 GB free"));
    assert_eq!(exec.run_calls.lock().unwrap().len(), 1, "no same-plan retry for a full disk");
    let health = crate::health::check(&app).await;
    let paused = health.iter().find(|h| h.source == "downloads").expect("hold surfaced in health");
    assert!(paused.message.starts_with("Downloads paused: disk full"), "{}", paused.message);

    settle(&app).await;
    settle(&app).await;
    let r = row(&app, "B2").await.unwrap();
    assert_eq!(r.state(), QueueState::Waiting);
    assert_eq!(r.waiting_reason.as_deref(), Some("Downloads paused: disk full"));
    assert_eq!(exec.plan_calls.lock().unwrap().len(), 1, "no licence requests while paused");

    // Free space hasn't grown, so the health re-check keeps the hold...
    assert!(!crate::health::try_release_hold(&app, StdDuration::ZERO).await);
    // ...until the user retries the failed download.
    enqueue(&app, "B1", Priority::Manual, false).await.unwrap();
    assert!(app.queue.hold().is_none());
    settle(&app).await;
    settle(&app).await;
    assert_eq!(row(&app, "B1").await.unwrap().state(), QueueState::Completed);
    assert_eq!(row(&app, "B2").await.unwrap().state(), QueueState::Completed);
}

#[tokio::test]
async fn a_broken_ffmpeg_pauses_the_queue_until_the_health_check_sees_it_working() {
    let exec = Arc::new(FakeExecutor::default());
    exec.runs.lock().unwrap().push_back(Err(PipelineError::new(FailureKind::Ffmpeg, "ffmpeg not found")));
    *exec.tools.lock().unwrap() = Some(Err("ffmpeg not found".into()));
    let (app, _dir) = test_app_with(exec.clone()).await;
    add_account(&app, "chirp:1", "chirp").await;
    add_book(&app, "chirp:1", "chirp", "a", "A").await;
    add_book(&app, "chirp:1", "chirp", "b", "B").await;
    enqueue(&app, "a", Priority::New, false).await.unwrap();
    enqueue(&app, "b", Priority::New, false).await.unwrap();
    settle(&app).await;
    assert_eq!(row(&app, "a").await.unwrap().state(), QueueState::Failed);
    settle(&app).await;
    assert_eq!(row(&app, "b").await.unwrap().waiting_reason.as_deref(), Some("Downloads paused: ffmpeg is not working"));

    assert!(!crate::health::try_release_hold(&app, StdDuration::ZERO).await, "still broken");
    *exec.tools.lock().unwrap() = None;
    assert!(crate::health::try_release_hold(&app, StdDuration::ZERO).await);
    settle(&app).await;
    assert_eq!(row(&app, "b").await.unwrap().state(), QueueState::Completed);
    assert_eq!(row(&app, "a").await.unwrap().state(), QueueState::Failed, "the failed item waits for a retry");
}

#[tokio::test]
async fn a_network_blip_retries_with_the_same_plan() {
    let exec = Arc::new(FakeExecutor::default());
    exec.runs.lock().unwrap().push_back(Err(PipelineError::new(FailureKind::Network, "connection reset")));
    let (app, _dir) = test_app_with(exec.clone()).await;
    add_account(&app, "amzn:us", "audible").await;
    add_book(&app, "amzn:us", "audible", "B1", "One").await;
    enqueue(&app, "B1", Priority::New, false).await.unwrap();
    settle(&app).await;
    let r = row(&app, "B1").await.unwrap();
    assert_eq!((r.state(), r.attempts), (QueueState::Completed, 0));
    assert_eq!(exec.plan_calls.lock().unwrap().len(), 1, "one licence");
    assert_eq!(exec.run_calls.lock().unwrap().len(), 2, "two pipeline runs");
}

#[tokio::test]
async fn audible_unavailable_holds_the_whole_customer() {
    let exec = Arc::new(FakeExecutor::default());
    exec.plans.lock().unwrap().push_back(Err(PlanFailure {
        kind: PlanFailureKind::Unavailable,
        message: "HTTP 503".into(),
    }));
    let (app, _dir) = test_app_with(exec.clone()).await;
    add_account(&app, "amzn:us", "audible").await;
    add_book(&app, "amzn:us", "audible", "B1", "One").await;
    add_book(&app, "amzn:us", "audible", "B2", "Two").await;
    enqueue(&app, "B1", Priority::New, false).await.unwrap();
    enqueue(&app, "B2", Priority::New, false).await.unwrap();
    settle(&app).await;
    settle(&app).await;

    assert_eq!(exec.plan_calls.lock().unwrap().as_slice(), ["B1"]);
    let hold = crate::time::parse(&pacing_row(&app, "amzn").await.2.unwrap()).unwrap();
    let minutes = (hold - crate::time::now()).num_minutes();
    assert!((29..=30).contains(&minutes), "{minutes}");
    for b in ["B1", "B2"] {
        let r = row(&app, b).await.unwrap();
        assert_eq!(r.state(), QueueState::Waiting);
        assert!(r.waiting_reason.as_deref().unwrap().starts_with("Audible unavailable, retrying at"), "{b}: {:?}", r.waiting_reason);
    }
    let b1 = row(&app, "B1").await.unwrap();
    assert_eq!(b1.attempts, 1, "the title counts an attempt");
    assert!(b1.not_before().unwrap() >= hold);
    assert_eq!(row(&app, "B2").await.unwrap().attempts, 0);
}

#[tokio::test]
async fn books_that_render_to_one_folder_get_their_own() {
    let exec = Arc::new(FakeExecutor::default());
    let (app, _dir) = test_app_with(exec.clone()).await;
    set_concurrency(&app, 2).await;
    add_account(&app, "chirp:1", "chirp").await;
    for b in ["c1", "c2", "c3"] {
        add_book(&app, "chirp:1", "chirp", b, "Same Title").await;
    }
    enqueue(&app, "c1", Priority::New, false).await.unwrap();
    enqueue(&app, "c2", Priority::New, false).await.unwrap();
    settle(&app).await;

    let base = "Ann Author/Saga/2 - Same Title";
    let p1 = crate::books::get(&app.db, "c1").await.unwrap().unwrap().output_path.unwrap();
    let p2 = crate::books::get(&app.db, "c2").await.unwrap().unwrap().output_path.unwrap();
    let mut got = vec![p1, p2];
    got.sort();
    assert!(
        got == vec![base.to_string(), format!("{base} [c2]")] || got == vec![base.to_string(), format!("{base} [c1]")],
        "{got:?}"
    );
    let dirs = exec.run_calls.lock().unwrap().clone();
    assert_eq!(dirs.len(), 2);
    assert_ne!(dirs[0], dirs[1], "the two pipelines wrote to different folders");

    // A book saved earlier also keeps its folder.
    enqueue(&app, "c3", Priority::New, false).await.unwrap();
    settle(&app).await;
    let p3 = crate::books::get(&app.db, "c3").await.unwrap().unwrap().output_path.unwrap();
    assert_eq!(p3, format!("{base} [c3]"));

    // Downloading a book again reuses its own folder.
    enqueue(&app, "c3", Priority::Manual, false).await.unwrap();
    settle(&app).await;
    assert_eq!(crate::books::get(&app.db, "c3").await.unwrap().unwrap().output_path.unwrap(), format!("{base} [c3]"));
}

#[tokio::test]
async fn force_is_spent_by_the_licence_request_whatever_the_answer() {
    let exec = Arc::new(FakeExecutor::default());
    exec.plans.lock().unwrap().push_back(Err(PlanFailure { kind: PlanFailureKind::Other, message: "odd".into() }));
    let (app, _dir) = test_app_with(exec.clone()).await;
    add_account(&app, "amzn:us", "audible").await;
    add_book(&app, "amzn:us", "audible", "B1", "One").await;
    let r = enqueue(&app, "B1", Priority::Manual, true).await.unwrap();
    assert!(r.force);
    settle(&app).await;
    let r = row(&app, "B1").await.unwrap();
    assert_eq!(r.state(), QueueState::Waiting);
    assert!(!r.force);
}

#[tokio::test]
async fn worker_transitions_leave_an_item_someone_else_moved_alone() {
    let (app, _dir) = test_app_with(Arc::new(FakeExecutor::default())).await;
    add_account(&app, "chirp:1", "chirp").await;
    add_book(&app, "chirp:1", "chirp", "a", "A").await;
    let r = enqueue(&app, "a", Priority::New, false).await.unwrap();
    sqlx::query("UPDATE queue SET state = 'cancelled' WHERE id = ?").bind(r.id).execute(&app.db).await.unwrap();
    let outcome = super::worker::Outcome::Failed { message: "late".into(), retryable: true };
    super::worker::finish(&app, &r, outcome, false).await.unwrap();
    let after = get(&app.db, r.id).await.unwrap().unwrap();
    assert_eq!((after.state(), after.attempts), (QueueState::Cancelled, 0));
}

#[tokio::test]
async fn cancelling_only_touches_items_in_the_state_it_expects() {
    let (app, _dir) = test_app_with(Arc::new(FakeExecutor::default())).await;
    add_account(&app, "chirp:1", "chirp").await;
    add_book(&app, "chirp:1", "chirp", "a", "A").await;
    add_book(&app, "chirp:1", "chirp", "b", "B").await;
    let queued = enqueue(&app, "a", Priority::New, false).await.unwrap();
    let running = enqueue(&app, "b", Priority::New, false).await.unwrap();
    sqlx::query("UPDATE queue SET state = 'running' WHERE id = ?").bind(running.id).execute(&app.db).await.unwrap();
    let token = tokio_util::sync::CancellationToken::new();
    let user = Arc::new(AtomicBool::new(false));
    app.queue.register(running.id, token.clone(), user.clone());

    let cancel = |id: i64| {
        axum::http::Request::delete(format!("/api/v1/queue/{id}"))
            .header("X-Api-Key", app.api_key())
            .body(axum::body::Body::empty())
            .unwrap()
    };
    for id in [queued.id, running.id] {
        let resp = tower::ServiceExt::oneshot(crate::api::router(app.clone()), cancel(id)).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NO_CONTENT);
    }
    assert_eq!(get(&app.db, queued.id).await.unwrap().unwrap().state(), QueueState::Cancelled);
    // The running one is signalled; its worker records the cancellation.
    assert_eq!(get(&app.db, running.id).await.unwrap().unwrap().state(), QueueState::Running);
    assert!(token.is_cancelled() && user.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn a_stale_worker_cannot_unregister_or_clear_a_newer_one() {
    let rt = Runtime::default();
    let first = rt.register(7, tokio_util::sync::CancellationToken::new(), Arc::new(AtomicBool::new(false)));
    let second = rt.register(7, tokio_util::sync::CancellationToken::new(), Arc::new(AtomicBool::new(false)));
    rt.unregister(7, first);
    assert!(rt.is_running(7), "the newer registration stays");
    rt.unregister(7, second);
    assert!(!rt.is_running(7));

    rt.set_licence_in_flight("amzn", 1);
    rt.set_licence_in_flight("amzn", 2);
    rt.clear_licence_in_flight("amzn", 1);
    assert!(rt.licence_in_flight("amzn"), "item 1 can't clear item 2's request");
    rt.clear_licence_in_flight("amzn", 2);
    assert!(!rt.licence_in_flight("amzn"));
}

/// Insert `n` new chirp books, as a sync that started at `started` would have.
async fn sync_found(app: &AppState, account: &str, ids: &[&str]) {
    for id in ids {
        add_book(app, account, "chirp", id, id).await;
    }
}

fn clean_sync(items: i64) -> crate::sync::PagedOutcome {
    crate::sync::PagedOutcome { pages_ok: 1, items, added: items, ..Default::default() }
}

async fn baseline(app: &AppState, account: &str) -> Option<String> {
    sqlx::query_scalar("SELECT baseline_at FROM account_state WHERE account_id = ?")
        .bind(account)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn only_a_complete_first_sync_sets_the_baseline() {
    let (app, _dir) = test_app_with(Arc::new(FakeExecutor::default())).await;
    add_account(&app, "chirp:1", "chirp").await;
    let started = crate::time::core_fmt(crate::time::now() - chrono::Duration::seconds(2));
    sync_found(&app, "chirp:1", &["a", "b"]).await;

    let skipped = crate::sync::PagedOutcome { pages_skipped: vec![(2, "boom".into())], ..clean_sync(2) };
    assert!(skipped.ok(), "a skipped page alone doesn't fail a sync");
    crate::sync::finish(&app, "chirp:1", "chirp", &started, &skipped).await.unwrap();
    assert_eq!(baseline(&app, "chirp:1").await, None, "a skipped page");

    let unreadable = crate::sync::PagedOutcome { items_failed: 1, ..clean_sync(2) };
    crate::sync::finish(&app, "chirp:1", "chirp", &started, &unreadable).await.unwrap();
    assert_eq!(baseline(&app, "chirp:1").await, None, "an unreadable item");

    crate::sync::finish(&app, "chirp:1", "chirp", &started, &clean_sync(2)).await.unwrap();
    assert!(baseline(&app, "chirp:1").await.is_some());
    let new_items = list_pending(&app.db).await.unwrap().into_iter().filter(|r| r.priority() == Priority::New).count();
    assert_eq!(new_items, 0, "the baseline's books are back-fill, not new");
}

#[tokio::test]
async fn a_library_that_never_syncs_completely_still_gets_a_baseline() {
    let (app, _dir) = test_app_with(Arc::new(FakeExecutor::default())).await;
    add_account(&app, "chirp:1", "chirp").await;
    let started = crate::time::core_fmt(crate::time::now() - chrono::Duration::seconds(2));
    let skipped = crate::sync::PagedOutcome { pages_skipped: vec![(3, "always fails".into())], ..clean_sync(2) };
    for _ in 1..crate::sync::BASELINE_ATTEMPTS {
        crate::sync::finish(&app, "chirp:1", "chirp", &started, &skipped).await.unwrap();
        assert_eq!(baseline(&app, "chirp:1").await, None);
    }
    // A failed sync doesn't count towards it.
    let failed = crate::sync::PagedOutcome { fatal: Some("token rejected".into()), ..Default::default() };
    crate::sync::finish(&app, "chirp:1", "chirp", &started, &failed).await.unwrap();
    assert_eq!(baseline(&app, "chirp:1").await, None);

    crate::sync::finish(&app, "chirp:1", "chirp", &started, &skipped).await.unwrap();
    assert!(baseline(&app, "chirp:1").await.is_some(), "set after {} incomplete syncs", crate::sync::BASELINE_ATTEMPTS);
    assert!(events(&app, "syncCompleted").await.iter().any(|e| e.message.starts_with("Library baseline set although")));
}

#[tokio::test]
async fn one_sync_queues_at_most_ten_books_as_new() {
    let (app, _dir) = test_app_with(Arc::new(FakeExecutor::default())).await;
    add_account(&app, "chirp:1", "chirp").await;
    sqlx::query("UPDATE account_state SET baseline_at = ?, backfill = 0")
        .bind(crate::time::fmt(crate::time::now() - chrono::Duration::days(1)))
        .execute(&app.db)
        .await
        .unwrap();
    let started = crate::time::core_fmt(crate::time::now() - chrono::Duration::seconds(2));
    let ids: Vec<String> = (1..=12).map(|i| format!("n{i:02}")).collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    sync_found(&app, "chirp:1", &refs).await;
    crate::sync::finish(&app, "chirp:1", "chirp", &started, &clean_sync(12)).await.unwrap();

    let pending = list_pending(&app.db).await.unwrap();
    assert_eq!(pending.iter().filter(|r| r.priority() == Priority::New).count(), crate::sync::MAX_NEW_PER_SYNC);
    assert_eq!(events(&app, "bookAdded").await.len(), crate::sync::MAX_NEW_PER_SYNC);
    let notes = events(&app, "syncCompleted").await;
    assert!(
        notes.iter().any(|n| n.message.contains("12 books appeared in one sync") && n.message.contains("leaving 2 to back-fill")),
        "{:?}",
        notes.iter().map(|n| &n.message).collect::<Vec<_>>()
    );
}
