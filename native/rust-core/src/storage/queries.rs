//! Database query functions
//!
//! This module implements repository pattern for database operations.
//! Ported from Libation's DbContext query methods and service layer.
//!
//! # Reference C# Sources
//! - `DataLayer/LibationContext.cs` - Main DbContext with entity sets
//! - `DataLayer/QueryObjects/BookQueries.cs` - Book query extensions
//! - `DtoImporterService/*.cs` - Import/upsert logic
//!
//! # Query Patterns
//! - Repository pattern per entity type
//! - Async/await for all database operations
//! - Use sqlx for type-safe queries
//! - Support transactions for multi-step operations

use crate::error::{LibationError, Result};
use crate::storage::models::*;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Executor, SqlitePool};

// ============================================================================
// BOOK QUERIES
// ============================================================================

/// Insert a new book
///
/// Returns the book_id of the inserted book.
pub async fn insert_book(pool: &SqlitePool, book: &NewBook) -> Result<i64> {
    let result = sqlx::query(
        r#"
        INSERT INTO Books (
            audible_product_id, title, subtitle, description, length_in_minutes,
            content_type, locale, picture_id, picture_large,
            is_abridged, is_spatial, date_published, language,
            rating_overall, rating_performance, rating_story
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&book.audible_product_id)
    .bind(&book.title)
    .bind(&book.subtitle)
    .bind(&book.description)
    .bind(book.length_in_minutes)
    .bind(book.content_type)
    .bind(&book.locale)
    .bind(&book.picture_id)
    .bind(&book.picture_large)
    .bind(book.is_abridged)
    .bind(book.is_spatial)
    .bind(book.date_published)
    .bind(&book.language)
    .bind(book.rating_overall)
    .bind(book.rating_performance)
    .bind(book.rating_story)
    .execute(pool)
    .await?;

    Ok(result.last_insert_rowid())
}

/// Find book by ASIN (audible product ID)
pub async fn find_book_by_asin(pool: &SqlitePool, asin: &str) -> Result<Option<Book>> {
    let book = sqlx::query_as::<_, Book>("SELECT * FROM Books WHERE audible_product_id = ?")
        .bind(asin)
        .fetch_optional(pool)
        .await?;

    Ok(book)
}

/// Find book by ID
pub async fn find_book_by_id(pool: &SqlitePool, book_id: i64) -> Result<Option<Book>> {
    let book = sqlx::query_as::<_, Book>("SELECT * FROM Books WHERE book_id = ?")
        .bind(book_id)
        .fetch_optional(pool)
        .await?;

    Ok(book)
}

/// Update an existing book
pub async fn update_book(pool: &SqlitePool, book: &Book) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE Books SET
            title = ?, subtitle = ?, description = ?, length_in_minutes = ?,
            content_type = ?, picture_id = ?, picture_large = ?,
            is_abridged = ?, is_spatial = ?, date_published = ?, language = ?,
            rating_overall = ?, rating_performance = ?, rating_story = ?
        WHERE book_id = ?
        "#,
    )
    .bind(&book.title)
    .bind(&book.subtitle)
    .bind(&book.description)
    .bind(book.length_in_minutes)
    .bind(book.content_type)
    .bind(&book.picture_id)
    .bind(&book.picture_large)
    .bind(book.is_abridged)
    .bind(book.is_spatial)
    .bind(book.date_published)
    .bind(&book.language)
    .bind(book.rating_overall)
    .bind(book.rating_performance)
    .bind(book.rating_story)
    .bind(book.book_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// List all books with pagination (basic - no relations)
pub async fn list_books(pool: &SqlitePool, limit: i64, offset: i64) -> Result<Vec<Book>> {
    let books = sqlx::query_as::<_, Book>(
        "SELECT * FROM Books ORDER BY title LIMIT ? OFFSET ?",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(books)
}

/// Enhanced book data with all relationships included
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BookWithRelations {
    // Core book fields
    pub book_id: i64,
    pub audible_product_id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub description: String,
    pub length_in_minutes: i32,
    pub content_type: i32,
    pub locale: String,
    pub picture_id: Option<String>,
    pub picture_large: Option<String>,
    pub is_abridged: bool,
    pub is_spatial: bool,
    pub date_published: Option<String>,
    pub language: Option<String>,
    pub rating_overall: f32,
    pub rating_performance: f32,
    pub rating_story: f32,
    pub pdf_url: Option<String>,
    pub is_finished: bool,
    pub is_downloadable: bool,
    pub is_ayce: bool,
    pub origin_asin: Option<String>,
    pub episode_number: Option<i32>,
    pub content_delivery_type: Option<String>,
    pub created_at: String,
    pub updated_at: String,

    // Related data (comma-separated strings)
    pub authors_str: Option<String>,
    pub narrators_str: Option<String>,
    pub publisher: Option<String>,
    pub series_name: Option<String>,
    pub series_sequence: Option<f32>,
    pub purchase_date: Option<String>,
}

/// List books with all related data (authors, narrators, series, etc.)
pub async fn list_books_with_relations(pool: &SqlitePool, limit: i64, offset: i64) -> Result<Vec<BookWithRelations>> {
    let books = sqlx::query_as::<_, BookWithRelations>(
        r#"
        WITH book_authors AS (
            SELECT
                bc.book_id,
                GROUP_CONCAT(c.name, ', ') as authors
            FROM BookContributors bc
            JOIN Contributors c ON bc.contributor_id = c.contributor_id
            WHERE bc.role = 1
            GROUP BY bc.book_id
        ),
        book_narrators AS (
            SELECT
                bc.book_id,
                GROUP_CONCAT(c.name, ', ') as narrators
            FROM BookContributors bc
            JOIN Contributors c ON bc.contributor_id = c.contributor_id
            WHERE bc.role = 2
            GROUP BY bc.book_id
        ),
        book_publishers AS (
            SELECT
                bc.book_id,
                c.name as publisher
            FROM BookContributors bc
            JOIN Contributors c ON bc.contributor_id = c.contributor_id
            WHERE bc.role = 3
        ),
        book_series AS (
            SELECT
                sb.book_id,
                s.name as series_name,
                sb."index" as series_sequence,
                ROW_NUMBER() OVER (PARTITION BY sb.book_id ORDER BY sb."index") as rn
            FROM SeriesBooks sb
            JOIN Series s ON sb.series_id = s.series_id
        )
        SELECT
            b.book_id,
            b.audible_product_id,
            b.title,
            b.subtitle,
            b.description,
            b.length_in_minutes,
            b.content_type,
            b.locale,
            b.picture_id,
            b.picture_large,
            b.is_abridged,
            b.is_spatial,
            b.date_published,
            b.language,
            b.rating_overall,
            b.rating_performance,
            b.rating_story,
            b.pdf_url,
            b.is_finished,
            b.is_downloadable,
            b.is_ayce,
            b.origin_asin,
            b.episode_number,
            b.content_delivery_type,
            b.created_at,
            b.updated_at,
            ba.authors as authors_str,
            bn.narrators as narrators_str,
            bp.publisher,
            bs.series_name,
            bs.series_sequence,
            lb.date_added as purchase_date
        FROM Books b
        LEFT JOIN book_authors ba ON b.book_id = ba.book_id
        LEFT JOIN book_narrators bn ON b.book_id = bn.book_id
        LEFT JOIN book_publishers bp ON b.book_id = bp.book_id
        LEFT JOIN book_series bs ON b.book_id = bs.book_id AND bs.rn = 1
        LEFT JOIN LibraryBooks lb ON b.book_id = lb.book_id
        ORDER BY b.title
        LIMIT ? OFFSET ?
        "#,
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(books)
}

/// Count total books
pub async fn count_books(pool: &SqlitePool) -> Result<i64> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM Books")
        .fetch_one(pool)
        .await?;

    Ok(count)
}

/// Search books by title
pub async fn search_books_by_title(pool: &SqlitePool, query: &str, limit: i64) -> Result<Vec<Book>> {
    let search_pattern = format!("%{}%", query);
    let books = sqlx::query_as::<_, Book>(
        "SELECT * FROM Books WHERE title LIKE ? OR subtitle LIKE ? ORDER BY title LIMIT ?",
    )
    .bind(&search_pattern)
    .bind(&search_pattern)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(books)
}

/// Delete a book (and all related data via CASCADE)
pub async fn delete_book(pool: &SqlitePool, book_id: i64) -> Result<()> {
    sqlx::query("DELETE FROM Books WHERE book_id = ?")
        .bind(book_id)
        .execute(pool)
        .await?;

    Ok(())
}

// ============================================================================
// LIBRARY BOOK QUERIES
// ============================================================================

/// Insert a new library book entry
pub async fn insert_library_book(pool: &SqlitePool, library_book: &NewLibraryBook) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO LibraryBooks (book_id, account)
        VALUES (?, ?)
        "#,
    )
    .bind(library_book.book_id)
    .bind(&library_book.account)
    .execute(pool)
    .await?;

    Ok(())
}

/// Find library book by book_id
pub async fn find_library_book(pool: &SqlitePool, book_id: i64) -> Result<Option<LibraryBook>> {
    let lib_book = sqlx::query_as::<_, LibraryBook>("SELECT * FROM LibraryBooks WHERE book_id = ?")
        .bind(book_id)
        .fetch_optional(pool)
        .await?;

    Ok(lib_book)
}

/// List all library books for an account
pub async fn list_library_books_by_account(pool: &SqlitePool, account: &str) -> Result<Vec<LibraryBook>> {
    let books = sqlx::query_as::<_, LibraryBook>(
        "SELECT * FROM LibraryBooks WHERE account = ? AND is_deleted = 0 ORDER BY date_added DESC",
    )
    .bind(account)
    .fetch_all(pool)
    .await?;

    Ok(books)
}

// ============================================================================
// USER DEFINED ITEM QUERIES
// ============================================================================

/// Insert a new user defined item
pub async fn insert_user_defined_item(pool: &SqlitePool, item: &NewUserDefinedItem) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO UserDefinedItems (book_id)
        VALUES (?)
        "#,
    )
    .bind(item.book_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Find user defined item by book_id
pub async fn find_user_defined_item(pool: &SqlitePool, book_id: i64) -> Result<Option<UserDefinedItem>> {
    let item = sqlx::query_as::<_, UserDefinedItem>("SELECT * FROM UserDefinedItems WHERE book_id = ?")
        .bind(book_id)
        .fetch_optional(pool)
        .await?;

    Ok(item)
}

/// Update user defined item
pub async fn update_user_defined_item(pool: &SqlitePool, item: &UserDefinedItem) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE UserDefinedItems SET
            tags = ?, user_rating_overall = ?, user_rating_performance = ?, user_rating_story = ?,
            book_status = ?, pdf_status = ?,
            last_downloaded = ?, last_downloaded_version = ?,
            last_downloaded_format = ?, last_downloaded_file_version = ?,
            is_finished = ?
        WHERE book_id = ?
        "#,
    )
    .bind(&item.tags)
    .bind(item.user_rating_overall)
    .bind(item.user_rating_performance)
    .bind(item.user_rating_story)
    .bind(item.book_status)
    .bind(item.pdf_status)
    .bind(item.last_downloaded)
    .bind(&item.last_downloaded_version)
    .bind(item.last_downloaded_format)
    .bind(&item.last_downloaded_file_version)
    .bind(item.is_finished)
    .bind(item.book_id)
    .execute(pool)
    .await?;

    Ok(())
}

// ============================================================================
// CONTRIBUTOR QUERIES
// ============================================================================

/// Insert or find contributor by name
///
/// Returns the contributor_id (either existing or newly created)
pub async fn upsert_contributor(pool: &SqlitePool, contributor: &NewContributor) -> Result<i64> {
    // Try to find existing contributor
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT contributor_id FROM Contributors WHERE name = ? AND (audible_contributor_id = ? OR (audible_contributor_id IS NULL AND ? IS NULL))"
    )
    .bind(&contributor.name)
    .bind(&contributor.audible_contributor_id)
    .bind(&contributor.audible_contributor_id)
    .fetch_optional(pool)
    .await?;

    if let Some(id) = existing {
        return Ok(id);
    }

    // Insert new contributor
    let result = sqlx::query(
        "INSERT INTO Contributors (name, audible_contributor_id) VALUES (?, ?)",
    )
    .bind(&contributor.name)
    .bind(&contributor.audible_contributor_id)
    .execute(pool)
    .await?;

    Ok(result.last_insert_rowid())
}

/// Find contributors by book ID and role
pub async fn find_contributors_by_book(pool: &SqlitePool, book_id: i64, role: i32) -> Result<Vec<Contributor>> {
    let contributors = sqlx::query_as::<_, Contributor>(
        r#"
        SELECT c.* FROM Contributors c
        INNER JOIN BookContributors bc ON c.contributor_id = bc.contributor_id
        WHERE bc.book_id = ? AND bc.role = ?
        ORDER BY bc."order"
        "#,
    )
    .bind(book_id)
    .bind(role)
    .fetch_all(pool)
    .await?;

    Ok(contributors)
}

/// Link book to contributor
pub async fn add_book_contributor(
    pool: &SqlitePool,
    book_id: i64,
    contributor_id: i64,
    role: i32,
    order: i16,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT OR REPLACE INTO BookContributors (book_id, contributor_id, role, "order")
        VALUES (?, ?, ?, ?)
        "#,
    )
    .bind(book_id)
    .bind(contributor_id)
    .bind(role)
    .bind(order)
    .execute(pool)
    .await?;

    Ok(())
}

/// Remove all contributors of a specific role from a book
pub async fn remove_book_contributors_by_role(pool: &SqlitePool, book_id: i64, role: i32) -> Result<()> {
    sqlx::query("DELETE FROM BookContributors WHERE book_id = ? AND role = ?")
        .bind(book_id)
        .bind(role)
        .execute(pool)
        .await?;

    Ok(())
}

// ============================================================================
// SERIES QUERIES
// ============================================================================

/// Insert or find series by audible series ID
pub async fn upsert_series(pool: &SqlitePool, series: &NewSeries) -> Result<i64> {
    // Try to find existing series
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT series_id FROM Series WHERE audible_series_id = ?",
    )
    .bind(&series.audible_series_id)
    .fetch_optional(pool)
    .await?;

    if let Some(id) = existing {
        // Update name if provided
        if let Some(name) = &series.name {
            sqlx::query("UPDATE Series SET name = ? WHERE series_id = ?")
                .bind(name)
                .bind(id)
                .execute(pool)
                .await?;
        }
        return Ok(id);
    }

    // Insert new series
    let result = sqlx::query(
        "INSERT INTO Series (audible_series_id, name) VALUES (?, ?)",
    )
    .bind(&series.audible_series_id)
    .bind(&series.name)
    .execute(pool)
    .await?;

    Ok(result.last_insert_rowid())
}

/// Link book to series
pub async fn add_book_to_series(
    pool: &SqlitePool,
    series_id: i64,
    book_id: i64,
    order: Option<String>,
    index: f32,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT OR REPLACE INTO SeriesBooks (series_id, book_id, "order", "index")
        VALUES (?, ?, ?, ?)
        "#,
    )
    .bind(series_id)
    .bind(book_id)
    .bind(order)
    .bind(index)
    .execute(pool)
    .await?;

    Ok(())
}

/// Find series for a book
pub async fn find_series_by_book(pool: &SqlitePool, book_id: i64) -> Result<Vec<(Series, SeriesBook)>> {
    let results = sqlx::query_as::<_, (i64, String, Option<String>, i64, i64, Option<String>, f32)>(
        r#"
        SELECT s.series_id, s.audible_series_id, s.name,
               sb.series_id, sb.book_id, sb."order", sb."index"
        FROM Series s
        INNER JOIN SeriesBooks sb ON s.series_id = sb.series_id
        WHERE sb.book_id = ?
        ORDER BY sb."index"
        "#,
    )
    .bind(book_id)
    .fetch_all(pool)
    .await?;

    let series_books = results
        .into_iter()
        .map(|(series_id, audible_series_id, name, sb_series_id, sb_book_id, order, index)| {
            let series = Series {
                series_id,
                audible_series_id,
                name,
            };
            let series_book = SeriesBook {
                series_id: sb_series_id,
                book_id: sb_book_id,
                order,
                index,
            };
            (series, series_book)
        })
        .collect();

    Ok(series_books)
}

// ============================================================================
// CATEGORY QUERIES
// ============================================================================

/// Upsert category
pub async fn upsert_category(pool: &SqlitePool, category: &NewCategory) -> Result<i64> {
    // Try to find existing category
    if let Some(ref audible_id) = category.audible_category_id {
        let existing: Option<i64> = sqlx::query_scalar(
            "SELECT category_id FROM Categories WHERE audible_category_id = ?",
        )
        .bind(audible_id)
        .fetch_optional(pool)
        .await?;

        if let Some(id) = existing {
            return Ok(id);
        }
    }

    // Insert new category
    let result = sqlx::query(
        "INSERT INTO Categories (audible_category_id, name) VALUES (?, ?)",
    )
    .bind(&category.audible_category_id)
    .bind(&category.name)
    .execute(pool)
    .await?;

    Ok(result.last_insert_rowid())
}

/// Upsert category ladder
pub async fn upsert_category_ladder(pool: &SqlitePool, ladder: &NewCategoryLadder) -> Result<i64> {
    // Try to find existing ladder
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT category_ladder_id FROM CategoryLadders WHERE audible_ladder_id = ?",
    )
    .bind(&ladder.audible_ladder_id)
    .fetch_optional(pool)
    .await?;

    if let Some(id) = existing {
        return Ok(id);
    }

    // Insert new ladder
    let result = sqlx::query(
        "INSERT INTO CategoryLadders (audible_ladder_id, ladder) VALUES (?, ?)",
    )
    .bind(&ladder.audible_ladder_id)
    .bind(&ladder.ladder)
    .execute(pool)
    .await?;

    Ok(result.last_insert_rowid())
}

/// Link book to category ladder
pub async fn add_book_category(pool: &SqlitePool, book_id: i64, category_ladder_id: i64) -> Result<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO BookCategories (book_id, category_ladder_id) VALUES (?, ?)",
    )
    .bind(book_id)
    .bind(category_ladder_id)
    .execute(pool)
    .await?;

    Ok(())
}

// ============================================================================
// SUPPLEMENT QUERIES
// ============================================================================

/// Add supplement to book
pub async fn add_supplement(pool: &SqlitePool, book_id: i64, url: &str) -> Result<i64> {
    let result = sqlx::query(
        "INSERT INTO Supplements (book_id, url) VALUES (?, ?)",
    )
    .bind(book_id)
    .bind(url)
    .execute(pool)
    .await?;

    Ok(result.last_insert_rowid())
}

/// Find supplements for a book
pub async fn find_supplements_by_book(pool: &SqlitePool, book_id: i64) -> Result<Vec<Supplement>> {
    let supplements = sqlx::query_as::<_, Supplement>(
        "SELECT * FROM Supplements WHERE book_id = ?",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await?;

    Ok(supplements)
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Upsert book (insert or update if ASIN exists)
///
/// This is a common operation when syncing from Audible API.
/// Returns the book_id (either existing or newly created).
pub async fn upsert_book(pool: &SqlitePool, book: &NewBook) -> Result<i64> {
    // Check if book exists
    if let Some(existing) = find_book_by_asin(pool, &book.audible_product_id).await? {
        // Update existing book
        let mut updated = existing;
        updated.title = book.title.clone();
        updated.subtitle = book.subtitle.clone();
        updated.description = book.description.clone();
        updated.length_in_minutes = book.length_in_minutes;
        updated.content_type = book.content_type;
        updated.picture_id = book.picture_id.clone();
        updated.picture_large = book.picture_large.clone();
        updated.is_abridged = book.is_abridged;
        updated.is_spatial = book.is_spatial;
        updated.date_published = book.date_published;
        updated.language = book.language.clone();
        updated.rating_overall = book.rating_overall;
        updated.rating_performance = book.rating_performance;
        updated.rating_story = book.rating_story;
        updated.updated_at = Utc::now();

        update_book(pool, &updated).await?;
        Ok(updated.book_id)
    } else {
        // Insert new book
        let book_id = insert_book(pool, book).await?;

        // Create default UserDefinedItem for the book
        insert_user_defined_item(pool, &NewUserDefinedItem::new(book_id)).await?;

        Ok(book_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::database::Database;

    #[tokio::test]
    async fn test_insert_and_find_book() {
        let db = Database::new_in_memory().await.expect("Failed to create database");

        let new_book = NewBook::new(
            "B012345678".to_string(),
            "Test Book".to_string(),
            "us".to_string(),
        );

        let book_id = insert_book(db.pool(), &new_book).await.expect("Failed to insert book");
        assert!(book_id > 0);

        let found = find_book_by_asin(db.pool(), "B012345678")
            .await
            .expect("Failed to find book");

        assert!(found.is_some());
        let book = found.unwrap();
        assert_eq!(book.title, "Test Book");
        assert_eq!(book.audible_product_id, "B012345678");
    }

    #[tokio::test]
    async fn test_upsert_book() {
        let db = Database::new_in_memory().await.expect("Failed to create database");

        let new_book = NewBook::new(
            "B012345679".to_string(),
            "Test Book Original".to_string(),
            "us".to_string(),
        );

        // First upsert - should insert
        let book_id1 = upsert_book(db.pool(), &new_book).await.expect("Failed to upsert book");

        // Second upsert with same ASIN - should update
        let mut updated_book = new_book.clone();
        updated_book.title = "Test Book Updated".to_string();
        let book_id2 = upsert_book(db.pool(), &updated_book).await.expect("Failed to upsert book");

        assert_eq!(book_id1, book_id2, "Book ID should be the same on update");

        let found = find_book_by_id(db.pool(), book_id1).await.expect("Failed to find book");
        assert_eq!(found.unwrap().title, "Test Book Updated");
    }

    #[tokio::test]
    async fn test_contributor_operations() {
        let db = Database::new_in_memory().await.expect("Failed to create database");

        let contributor = NewContributor::new("Test Author".to_string());
        let contributor_id = upsert_contributor(db.pool(), &contributor)
            .await
            .expect("Failed to upsert contributor");

        assert!(contributor_id > 0);

        // Upserting again should return same ID
        let contributor_id2 = upsert_contributor(db.pool(), &contributor)
            .await
            .expect("Failed to upsert contributor");

        assert_eq!(contributor_id, contributor_id2);
    }
}
