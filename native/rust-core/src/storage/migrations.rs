// LibriSync - Audible Library Sync for Mobile
// Copyright (C) 2025 Henning Berge
//
// This program is a Rust port of Libation (https://github.com/rmcrackan/Libation)
// Original work Copyright (C) Libation contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

//! Database migrations
//!
//! This module handles database schema creation and migrations.
//! Ported from Libation's Entity Framework migrations.
//!
//! # Reference C# Sources
//! - `DataLayer/Migrations/20191125182309_Fresh.cs` - Initial schema
//! - `DataLayer/Configurations/*.cs` - Table configurations
//!
//! # Migration Strategy
//! Since sqlx's compile-time migration system requires build-time database connection,
//! we implement migrations as runtime SQL execution for mobile compatibility.

use crate::error::Result;
use sqlx::{Executor, SqlitePool};

/// Run all database migrations
///
/// This function creates the database schema and applies any pending migrations.
/// Migrations are tracked in the `_migrations` table.
pub async fn run_migrations(pool: &SqlitePool) -> Result<()> {
    // Create migrations tracking table
    create_migrations_table(pool).await?;

    // Run all migrations in order
    run_migration(pool, 1, "initial_schema", create_initial_schema(pool)).await?;
    run_migration(pool, 2, "download_tasks", create_download_tasks_table(pool)).await?;
    run_migration(pool, 3, "accounts", create_accounts_table(pool)).await?;
    run_migration(
        pool,
        4,
        "download_conversion_columns",
        add_download_conversion_columns(pool),
    )
    .await?;
    run_migration(pool, 5, "add_source_column", add_source_column(pool)).await?;
    run_migration(pool, 6, "book_accounts", create_book_accounts_table(pool)).await?;
    run_migration(pool, 7, "add_provider_column", add_provider_column(pool)).await?;

    Ok(())
}

/// Create migrations tracking table
async fn create_migrations_table(pool: &SqlitePool) -> Result<()> {
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS _migrations (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        )
        "#,
    )
    .await?;

    Ok(())
}

/// Run a single migration if it hasn't been applied yet
async fn run_migration(
    pool: &SqlitePool,
    id: i32,
    name: &str,
    migration_fn: impl std::future::Future<Output = Result<()>>,
) -> Result<()> {
    // Check if migration has been applied
    let applied: Option<i32> = sqlx::query_scalar("SELECT id FROM _migrations WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;

    if applied.is_some() {
        // Migration already applied
        return Ok(());
    }

    // Run migration
    migration_fn.await?;

    // Record migration
    sqlx::query("INSERT INTO _migrations (id, name) VALUES (?, ?)")
        .bind(id)
        .bind(name)
        .execute(pool)
        .await?;

    Ok(())
}

/// Create initial database schema
///
/// Maps to C# Fresh migration (20191125182309_Fresh.cs)
/// Creates all tables with their relationships, indexes, and constraints.
async fn create_initial_schema(pool: &SqlitePool) -> Result<()> {
    // Execute all schema creation statements
    pool.execute(
        r#"
-- ============================================================================
-- MAIN ENTITIES
-- ============================================================================

-- Books table: Core audiobook metadata
-- Maps to C# Book entity in Book.cs
CREATE TABLE IF NOT EXISTS Books (
    book_id INTEGER PRIMARY KEY AUTOINCREMENT,

    -- Immutable core fields
    audible_product_id TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    subtitle TEXT,
    description TEXT NOT NULL DEFAULT '',
    length_in_minutes INTEGER NOT NULL,
    content_type INTEGER NOT NULL DEFAULT 1,  -- ContentType enum (Product=1, Episode=2, Parent=4)
    locale TEXT NOT NULL,

    -- Mutable metadata
    picture_id TEXT,
    picture_large TEXT,

    -- Book details
    is_abridged INTEGER NOT NULL DEFAULT 0,
    is_spatial INTEGER NOT NULL DEFAULT 0,
    date_published TEXT,  -- ISO 8601 date (YYYY-MM-DD)
    language TEXT,

    -- Product rating (aggregate community rating - embedded Rating entity)
    rating_overall REAL NOT NULL DEFAULT 0.0,
    rating_performance REAL NOT NULL DEFAULT 0.0,
    rating_story REAL NOT NULL DEFAULT 0.0,

    -- Additional metadata from API
    pdf_url TEXT,  -- PDF companion file URL
    is_finished INTEGER NOT NULL DEFAULT 0,  -- Has user finished listening
    is_downloadable INTEGER NOT NULL DEFAULT 1,  -- Can be downloaded
    is_ayce INTEGER NOT NULL DEFAULT 0,  -- Audible Plus Catalog title
    origin_asin TEXT,  -- Original ASIN (for regional variants)
    episode_number INTEGER,  -- Episode number (for podcasts)
    content_delivery_type TEXT,  -- SinglePartBook, MultiPartBook, etc.

    -- Timestamps
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- LibraryBooks table: User ownership of books
-- Maps to C# LibraryBook entity in LibraryBook.cs
CREATE TABLE IF NOT EXISTS LibraryBooks (
    book_id INTEGER PRIMARY KEY,  -- 1:1 with Books, also primary key
    date_added TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    account TEXT NOT NULL,  -- Account ID/email
    is_deleted INTEGER NOT NULL DEFAULT 0,
    absent_from_last_scan INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (book_id) REFERENCES Books(book_id) ON DELETE CASCADE
);

-- UserDefinedItems table: User-specific metadata for books
-- Maps to C# UserDefinedItem entity in UserDefinedItem.cs (owned entity)
CREATE TABLE IF NOT EXISTS UserDefinedItems (
    book_id INTEGER PRIMARY KEY,  -- 1:1 with Books

    -- User tags (space-delimited, lowercase, alphanumeric + underscore)
    tags TEXT,

    -- User rating (personal, not aggregate - embedded Rating entity)
    user_rating_overall REAL NOT NULL DEFAULT 0.0,
    user_rating_performance REAL NOT NULL DEFAULT 0.0,
    user_rating_story REAL NOT NULL DEFAULT 0.0,

    -- Liberation status (LiberatedStatus enum: NotLiberated=0, Liberated=1, Error=2)
    book_status INTEGER NOT NULL DEFAULT 0,
    pdf_status INTEGER,  -- Nullable

    -- Download tracking
    last_downloaded TEXT,  -- ISO 8601 timestamp
    last_downloaded_version TEXT,  -- Libation version string
    last_downloaded_format INTEGER,  -- AudioFormat serialized as i64
    last_downloaded_file_version TEXT,  -- Audio file version string

    -- User state
    is_finished INTEGER NOT NULL DEFAULT 0,  -- Has user finished listening?

    FOREIGN KEY (book_id) REFERENCES Books(book_id) ON DELETE CASCADE
);

-- Contributors table: Authors, narrators, publishers
-- Maps to C# Contributor entity in Contributor.cs
CREATE TABLE IF NOT EXISTS Contributors (
    contributor_id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    audible_contributor_id TEXT,
    UNIQUE(name, audible_contributor_id)
);

-- Series table: Book series information
-- Maps to C# Series entity in Series.cs
CREATE TABLE IF NOT EXISTS Series (
    series_id INTEGER PRIMARY KEY AUTOINCREMENT,
    audible_series_id TEXT NOT NULL UNIQUE,
    name TEXT
);

-- Categories table: Genres and categories
-- Maps to C# Category entity in Category.cs
CREATE TABLE IF NOT EXISTS Categories (
    category_id INTEGER PRIMARY KEY AUTOINCREMENT,
    audible_category_id TEXT,
    name TEXT
);

-- CategoryLadders table: Hierarchical category paths
-- Maps to C# CategoryLadder entity in CategoryLadder.cs
CREATE TABLE IF NOT EXISTS CategoryLadders (
    category_ladder_id INTEGER PRIMARY KEY AUTOINCREMENT,
    audible_ladder_id TEXT NOT NULL UNIQUE,
    ladder TEXT NOT NULL  -- JSON array of category IDs representing the path
);

-- Supplements table: PDF supplements (author notes, companion PDFs)
-- Maps to C# Supplement entity in Supplement.cs (owned entity)
CREATE TABLE IF NOT EXISTS Supplements (
    supplement_id INTEGER PRIMARY KEY AUTOINCREMENT,
    book_id INTEGER NOT NULL,
    url TEXT NOT NULL,
    FOREIGN KEY (book_id) REFERENCES Books(book_id) ON DELETE CASCADE
);

-- ============================================================================
-- JUNCTION TABLES (Many-to-Many Relationships)
-- ============================================================================

-- BookContributors: Book <-> Contributor junction
-- Maps to C# BookContributor entity in BookContributor.cs
CREATE TABLE IF NOT EXISTS BookContributors (
    book_id INTEGER NOT NULL,
    contributor_id INTEGER NOT NULL,
    role INTEGER NOT NULL,  -- Role enum (Author=1, Narrator=2, Publisher=3)
    "order" INTEGER NOT NULL DEFAULT 0,  -- Order within role (quoted keyword)
    FOREIGN KEY (book_id) REFERENCES Books(book_id) ON DELETE CASCADE,
    FOREIGN KEY (contributor_id) REFERENCES Contributors(contributor_id) ON DELETE CASCADE,
    PRIMARY KEY (book_id, contributor_id, role)
);

-- SeriesBooks: Series <-> Book junction
-- Maps to C# SeriesBook entity in SeriesBook.cs
CREATE TABLE IF NOT EXISTS SeriesBooks (
    series_id INTEGER NOT NULL,
    book_id INTEGER NOT NULL,
    "order" TEXT,  -- Order string (e.g., "1", "2.5", "Book 3")
    "index" REAL NOT NULL DEFAULT 0.0,  -- Numeric index extracted from order string
    FOREIGN KEY (series_id) REFERENCES Series(series_id) ON DELETE CASCADE,
    FOREIGN KEY (book_id) REFERENCES Books(book_id) ON DELETE CASCADE,
    PRIMARY KEY (series_id, book_id)
);

-- BookCategories: Book <-> CategoryLadder junction
-- Maps to C# BookCategory entity in BookCategory.cs
CREATE TABLE IF NOT EXISTS BookCategories (
    book_id INTEGER NOT NULL,
    category_ladder_id INTEGER NOT NULL,
    FOREIGN KEY (book_id) REFERENCES Books(book_id) ON DELETE CASCADE,
    FOREIGN KEY (category_ladder_id) REFERENCES CategoryLadders(category_ladder_id) ON DELETE CASCADE,
    PRIMARY KEY (book_id, category_ladder_id)
);

-- ============================================================================
-- INDEXES for Performance
-- ============================================================================

-- Books indexes
CREATE INDEX IF NOT EXISTS idx_books_asin ON Books(audible_product_id);
CREATE INDEX IF NOT EXISTS idx_books_locale ON Books(locale);
CREATE INDEX IF NOT EXISTS idx_books_title ON Books(title);
CREATE INDEX IF NOT EXISTS idx_books_content_type ON Books(content_type);
CREATE INDEX IF NOT EXISTS idx_books_updated_at ON Books(updated_at);

-- LibraryBooks indexes
CREATE INDEX IF NOT EXISTS idx_library_books_account ON LibraryBooks(account);
CREATE INDEX IF NOT EXISTS idx_library_books_date_added ON LibraryBooks(date_added);
CREATE INDEX IF NOT EXISTS idx_library_books_is_deleted ON LibraryBooks(is_deleted);

-- Contributors indexes
CREATE INDEX IF NOT EXISTS idx_contributors_name ON Contributors(name);
CREATE INDEX IF NOT EXISTS idx_contributors_audible_id ON Contributors(audible_contributor_id);

-- BookContributors indexes
CREATE INDEX IF NOT EXISTS idx_book_contributors_book ON BookContributors(book_id, role, "order");
CREATE INDEX IF NOT EXISTS idx_book_contributors_contributor ON BookContributors(contributor_id);

-- Series indexes
CREATE INDEX IF NOT EXISTS idx_series_audible_id ON Series(audible_series_id);

-- SeriesBooks indexes
CREATE INDEX IF NOT EXISTS idx_series_books_series ON SeriesBooks(series_id, "index");
CREATE INDEX IF NOT EXISTS idx_series_books_book ON SeriesBooks(book_id);

-- Categories indexes
CREATE INDEX IF NOT EXISTS idx_categories_audible_id ON Categories(audible_category_id);

-- CategoryLadders indexes
CREATE INDEX IF NOT EXISTS idx_category_ladders_audible_id ON CategoryLadders(audible_ladder_id);

-- Supplements indexes
CREATE INDEX IF NOT EXISTS idx_supplements_book ON Supplements(book_id);

-- ============================================================================
-- TRIGGERS for Automatic Timestamp Updates
-- ============================================================================

-- Trigger to update updated_at timestamp when book is modified
CREATE TRIGGER IF NOT EXISTS update_books_timestamp
AFTER UPDATE ON Books
FOR EACH ROW
BEGIN
    UPDATE Books SET updated_at = CURRENT_TIMESTAMP WHERE book_id = NEW.book_id;
END;

-- ============================================================================
-- SEED DATA (for special cases)
-- ============================================================================

-- Insert empty contributor (matches C# -1 special case in migrations)
INSERT OR IGNORE INTO Contributors (contributor_id, name, audible_contributor_id)
VALUES (-1, '', NULL);

-- Insert empty category (matches C# -1 special case in migrations)
INSERT OR IGNORE INTO Categories (category_id, audible_category_id, name)
VALUES (-1, '', '');
        "#,
    )
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::database::Database;

    #[tokio::test]
    async fn test_migrations() {
        let db = Database::new_in_memory()
            .await
            .expect("Failed to create database");

        // Verify tables exist
        let tables: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name != '_migrations' ORDER BY name",
        )
        .fetch_all(db.pool())
        .await
        .expect("Failed to query tables");

        let expected_tables = vec![
            "Accounts",
            "BookAccounts",
            "BookCategories",
            "BookContributors",
            "Books",
            "Categories",
            "CategoryLadders",
            "Contributors",
            "DownloadTasks",
            "LibraryBooks",
            "Series",
            "SeriesBooks",
            "Supplements",
            "UserDefinedItems",
        ];

        assert_eq!(tables, expected_tables, "Missing or extra tables");
    }

    #[tokio::test]
    async fn test_migration_tracking() {
        let db = Database::new_in_memory()
            .await
            .expect("Failed to create database");

        // Verify migrations table exists and has records
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _migrations")
            .fetch_one(db.pool())
            .await
            .expect("Failed to query migrations");

        assert!(count > 0, "No migrations recorded");
    }

    #[tokio::test]
    async fn test_foreign_keys_enabled() {
        let db = Database::new_in_memory()
            .await
            .expect("Failed to create database");

        let fk_enabled: i32 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(db.pool())
            .await
            .expect("Failed to check foreign keys");

        assert_eq!(fk_enabled, 1, "Foreign keys not enabled");
    }
}

/// Create download_tasks table for Download Manager
///
/// This table stores persistent state for download operations including
/// queue position, partial download progress, and error states.
async fn create_download_tasks_table(pool: &SqlitePool) -> Result<()> {
    pool.execute(
        r#"
-- ============================================================================
-- DOWNLOAD MANAGER TABLES
-- ============================================================================

-- DownloadTasks table: Persistent download queue and state
CREATE TABLE IF NOT EXISTS DownloadTasks (
    task_id TEXT PRIMARY KEY,  -- UUID
    asin TEXT NOT NULL,
    title TEXT NOT NULL,
    status TEXT NOT NULL,  -- "queued", "downloading", "paused", "completed", "failed", "cancelled"

    -- Download progress
    bytes_downloaded INTEGER NOT NULL DEFAULT 0,
    total_bytes INTEGER NOT NULL DEFAULT 0,

    -- Download info
    download_url TEXT NOT NULL,
    download_path TEXT NOT NULL,  -- Cache path for encrypted file
    output_path TEXT NOT NULL,    -- Final path after decryption
    request_headers TEXT NOT NULL, -- JSON object with HTTP headers

    -- Error tracking
    error TEXT,
    retry_count INTEGER NOT NULL DEFAULT 0,

    -- Timestamps
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    started_at TEXT,
    completed_at TEXT
);

-- Indexes for common queries
CREATE INDEX IF NOT EXISTS idx_download_tasks_status ON DownloadTasks(status);
CREATE INDEX IF NOT EXISTS idx_download_tasks_asin ON DownloadTasks(asin);
CREATE INDEX IF NOT EXISTS idx_download_tasks_created_at ON DownloadTasks(created_at);
        "#,
    )
    .await?;

    Ok(())
}

/// Add conversion-related columns to DownloadTasks table
///
/// These columns store AAXC decryption keys and output directory so that
/// conversion can be retried without re-downloading the encrypted file.
async fn add_download_conversion_columns(pool: &SqlitePool) -> Result<()> {
    // SQLite doesn't support IF NOT EXISTS for ADD COLUMN, so check first
    let columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('DownloadTasks')")
            .fetch_all(pool)
            .await?;

    if !columns.contains(&"aaxc_key".to_string()) {
        pool.execute("ALTER TABLE DownloadTasks ADD COLUMN aaxc_key TEXT")
            .await?;
    }

    if !columns.contains(&"aaxc_iv".to_string()) {
        pool.execute("ALTER TABLE DownloadTasks ADD COLUMN aaxc_iv TEXT")
            .await?;
    }

    if !columns.contains(&"output_directory".to_string()) {
        pool.execute("ALTER TABLE DownloadTasks ADD COLUMN output_directory TEXT")
            .await?;
    }

    Ok(())
}

/// Create accounts table for storing user authentication data
///
/// This table stores account credentials, tokens, and device information.
/// Single source of truth accessible from both Rust and native workers.
async fn create_accounts_table(pool: &SqlitePool) -> Result<()> {
    pool.execute(
        r#"
-- ============================================================================
-- ACCOUNTS TABLE
-- ============================================================================

-- Accounts table: User authentication and identity
CREATE TABLE IF NOT EXISTS Accounts (
    account_id TEXT PRIMARY KEY,  -- Unique account identifier (email or username)
    account_name TEXT NOT NULL,
    locale_code TEXT NOT NULL,    -- Country code (e.g., "us", "uk", "de")

    -- Identity JSON (contains all auth data)
    -- Stores access_token, refresh_token, device info, cookies, etc.
    identity_json TEXT NOT NULL,

    -- Token expiry tracking
    token_expires_at TEXT,        -- ISO 8601 timestamp

    -- Account settings
    library_scan INTEGER NOT NULL DEFAULT 1,
    decrypt_key TEXT,             -- Activation bytes (8 hex chars)

    -- Timestamps
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_token_refresh TEXT,      -- Last successful token refresh
    last_library_sync TEXT        -- Last successful library sync
);

-- Index for quick lookup
CREATE INDEX IF NOT EXISTS idx_accounts_locale ON Accounts(locale_code);
CREATE INDEX IF NOT EXISTS idx_accounts_updated ON Accounts(updated_at);

-- Trigger to update updated_at timestamp
CREATE TRIGGER IF NOT EXISTS update_accounts_timestamp
AFTER UPDATE ON Accounts
FOR EACH ROW
BEGIN
    UPDATE Accounts SET updated_at = CURRENT_TIMESTAMP WHERE account_id = NEW.account_id;
END;
        "#,
    )
    .await?;

    Ok(())
}

/// Add source column to Books table for multi-source support (LibriVox, etc.)
///
/// This allows books from different sources (Audible, LibriVox) to coexist
/// in the same database. Existing books default to 'audible'.
async fn add_source_column(pool: &SqlitePool) -> Result<()> {
    let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('Books')")
        .fetch_all(pool)
        .await?;

    if !columns.contains(&"source".to_string()) {
        pool.execute("ALTER TABLE Books ADD COLUMN source TEXT NOT NULL DEFAULT 'audible'")
            .await?;
        pool.execute("CREATE INDEX IF NOT EXISTS idx_books_source ON Books(source)")
            .await?;
    }

    Ok(())
}

/// Tag each account with its provider (multi-provider support). Existing rows
/// default to 'audible', which is correct for all pre-Libro.fm accounts.
async fn add_provider_column(pool: &SqlitePool) -> Result<()> {
    let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('Accounts')")
        .fetch_all(pool)
        .await?;

    if !columns.contains(&"provider".to_string()) {
        pool.execute("ALTER TABLE Accounts ADD COLUMN provider TEXT NOT NULL DEFAULT 'audible'")
            .await?;
        pool.execute("CREATE INDEX IF NOT EXISTS idx_accounts_provider ON Accounts(provider)")
            .await?;
    }

    Ok(())
}

/// Add many-to-many account ownership for books.
///
/// LibraryBooks is kept for compatibility with older queries, while BookAccounts
/// stores one row per account/marketplace that owns a book.
async fn create_book_accounts_table(pool: &SqlitePool) -> Result<()> {
    pool.execute(
        r#"
CREATE TABLE IF NOT EXISTS BookAccounts (
    book_id INTEGER NOT NULL,
    account TEXT NOT NULL,
    date_added TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    is_deleted INTEGER NOT NULL DEFAULT 0,
    absent_from_last_scan INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (book_id) REFERENCES Books(book_id) ON DELETE CASCADE,
    PRIMARY KEY (book_id, account)
);

INSERT OR IGNORE INTO BookAccounts (
    book_id,
    account,
    date_added,
    is_deleted,
    absent_from_last_scan
)
SELECT
    book_id,
    account,
    date_added,
    is_deleted,
    absent_from_last_scan
FROM LibraryBooks;

CREATE INDEX IF NOT EXISTS idx_book_accounts_account ON BookAccounts(account);
CREATE INDEX IF NOT EXISTS idx_book_accounts_book ON BookAccounts(book_id);
CREATE INDEX IF NOT EXISTS idx_book_accounts_absent ON BookAccounts(account, absent_from_last_scan);
        "#,
    )
    .await?;

    Ok(())
}
