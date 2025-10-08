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
            "BookCategories",
            "BookContributors",
            "Books",
            "Categories",
            "CategoryLadders",
            "Contributors",
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
