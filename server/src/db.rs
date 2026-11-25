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

    let pool = SqlitePoolOptions::new()
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
        .with_context(|| format!("opening {}", server_db.display()))?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("running server.db migrations")?;
    Ok(pool)
}
