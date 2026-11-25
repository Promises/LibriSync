//! The server's own database (`server.db`), with the core DB attached as `core`.
//!
//! Attaching lets the books listing join the core's `Books`/`BookAccounts` with the
//! server's queue, download and backoff tables in one SQL statement, which keeps
//! filtering, sorting and paging in SQLite for large libraries. Writes to core tables
//! still go through rust-core's own functions and pool; this pool only reads them.

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

pub async fn open(server_db: &Path, core_db: &Path) -> Result<SqlitePool> {
    let pool = connect(server_db, core_db).await?;
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("running server.db migrations")?;
    Ok(pool)
}

/// The pool, without running migrations.
async fn connect(server_db: &Path, core_db: &Path) -> Result<SqlitePool> {
    let core_path = core_db
        .to_str()
        .context("core database path is not valid UTF-8")?
        .to_string();
    let opts = SqliteConnectOptions::from_str("sqlite://")?
        .filename(server_db)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(30))
        .foreign_keys(true);

    SqlitePoolOptions::new()
        .max_connections(8)
        .after_connect(move |conn, _meta| {
            let core_path = core_path.clone();
            Box::pin(async move {
                sqlx::query("ATTACH DATABASE ? AS core")
                    .bind(core_path)
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
        .connect_with(opts)
        .await
        .with_context(|| format!("opening {}", server_db.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Executor;

    /// A server.db written by the first release migrates to per-customer pacing: the two
    /// marketplaces of one customer merge into the stricter state, and Chirp's `chirp:…`
    /// ids (which contain a colon too) stay per account.
    #[tokio::test]
    async fn existing_pacing_state_moves_to_the_customer() {
        let dir = tempfile::tempdir().unwrap();
        let core_path = dir.path().join("core.db");
        let core = rust_core::storage::Database::new(&core_path).await.unwrap();
        for (id, provider) in [("amzn1.account.X:us", "audible"), ("chirp:7", "chirp")] {
            let json = serde_json::json!({ "account_id": id, "account_name": id, "provider": provider });
            rust_core::storage::accounts::save_account(core.pool(), id, &json.to_string()).await.unwrap();
        }
        let pool = connect(&dir.path().join("server.db"), &core_path).await.unwrap();
        pool.execute(include_str!("../migrations/0001_initial.sql")).await.unwrap();
        pool.execute(
            "INSERT INTO account_state (account_id, cooldown_until, cooldown_level, cooldown_reason, next_licence_at) VALUES
               ('amzn1.account.X:us', '2026-11-26T10:00:00.000Z', 1, 'throttle #1', '2026-11-25T10:00:00.000Z'),
               ('amzn1.account.X:uk', '2026-11-27T10:00:00.000Z', 2, 'throttle #2', NULL),
               ('chirp:7', NULL, 0, NULL, NULL);
             INSERT INTO licence_requests (account_id, book_id, time, outcome) VALUES
               ('amzn1.account.X:us', 'B1', '2026-11-25T09:00:00.000Z', 'granted'),
               ('amzn1.account.X:uk', 'B2', '2026-11-25T09:30:00.000Z', 'throttled');",
        )
        .await
        .unwrap();

        pool.execute(include_str!("../migrations/0002_customer_pacing.sql")).await.unwrap();

        let keys: Vec<(String, String)> =
            sqlx::query_as("SELECT account_id, pacing_key FROM account_state ORDER BY account_id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            keys,
            vec![
                // The UK account is no longer in the core DB; its id still names the customer.
                ("amzn1.account.X:uk".into(), "amzn1.account.X".into()),
                ("amzn1.account.X:us".into(), "amzn1.account.X".into()),
                ("chirp:7".into(), "chirp:7".into()),
            ]
        );
        let pacing: Vec<(String, Option<String>, i64)> =
            sqlx::query_as("SELECT pacing_key, cooldown_until, cooldown_level FROM licence_pacing ORDER BY pacing_key")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            pacing,
            vec![
                ("amzn1.account.X".into(), Some("2026-11-27T10:00:00.000Z".into()), 2),
                ("chirp:7".into(), None, 0),
            ]
        );
        let requests: Vec<String> = sqlx::query_scalar("SELECT pacing_key FROM licence_requests ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(requests, vec!["amzn1.account.X", "amzn1.account.X"]);
    }
}
