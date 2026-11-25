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
    for _ in 0..4 {
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
    let level: i64 = sqlx::query_scalar("SELECT cooldown_level FROM account_state WHERE account_id = 'amzn:us'")
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
    sqlx::query("INSERT INTO licence_requests (account_id, book_id, time, outcome) VALUES ('amzn:us', 'B0', ?, 'granted')")
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
