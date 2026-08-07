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
use std::collections::HashMap;

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
    let books = sqlx::query_as::<_, Book>("SELECT * FROM Books ORDER BY title LIMIT ? OFFSET ?")
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

    // Source (audible, librivox)
    #[sqlx(default)]
    pub source: Option<String>,

    // Related data (comma-separated strings)
    pub authors_str: Option<String>,
    pub narrators_str: Option<String>,
    pub publisher: Option<String>,
    pub series_name: Option<String>,
    pub series_sequence: Option<f32>,
    pub purchase_date: Option<String>,
    #[sqlx(default)]
    pub account: Option<String>,
}

impl BookWithRelations {
    /// Convert to AudioMetadata for path template rendering
    pub fn to_audio_metadata(&self) -> crate::audio::metadata::AudioMetadata {
        use crate::audio::metadata::{AudioMetadata, SeriesInfo};

        let authors = self
            .authors_str
            .as_ref()
            .map(|s| s.split(", ").map(String::from).collect())
            .unwrap_or_else(Vec::new);

        let narrators = self
            .narrators_str
            .as_ref()
            .map(|s| s.split(", ").map(String::from).collect())
            .unwrap_or_else(Vec::new);

        let series = if let Some(ref name) = self.series_name {
            Some(SeriesInfo {
                name: name.clone(),
                position: self.series_sequence.map(|seq| {
                    // Format sequence: 1.0 -> "1", 1.5 -> "1.5"
                    if seq.fract() == 0.0 {
                        format!("{:.0}", seq)
                    } else {
                        format!("{}", seq)
                    }
                }),
            })
        } else {
            None
        };

        AudioMetadata {
            title: self.title.clone(),
            authors,
            narrators,
            publisher: self.publisher.clone(),
            publication_date: self.date_published.clone(),
            language: self.language.clone(),
            series,
            description: Some(self.description.clone()),
            genres: vec![], // Not available in BookWithRelations
            runtime_minutes: Some(self.length_in_minutes),
            asin: Some(self.audible_product_id.clone()),
            cover_art_url: self.picture_large.clone(),
        }
    }
}

/// List books with all related data (authors, narrators, series, etc.)
pub async fn list_books_with_relations(
    pool: &SqlitePool,
    limit: i64,
    offset: i64,
) -> Result<Vec<BookWithRelations>> {
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
            COALESCE(b.source, 'audible') as source,
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

/// Get a single book with all related data by ASIN
pub async fn find_book_with_relations_by_asin(
    pool: &SqlitePool,
    asin: &str,
) -> Result<Option<BookWithRelations>> {
    let book = sqlx::query_as::<_, BookWithRelations>(
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
            COALESCE(b.source, 'audible') as source,
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
        WHERE b.audible_product_id = ?
        "#,
    )
    .bind(asin)
    .fetch_optional(pool)
    .await?;

    Ok(book)
}

/// Count total books
pub async fn count_books(pool: &SqlitePool) -> Result<i64> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM Books")
        .fetch_one(pool)
        .await?;

    Ok(count)
}

/// Sort options for book queries
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortField {
    Title,
    ReleaseDate,
    DateAdded,
    Series,
    Length,
    Downloaded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

fn downloaded_status_order_expression(direction: SortDirection) -> &'static str {
    match direction {
        SortDirection::Asc => "CASE WHEN completed_downloads.asin IS NULL THEN 0 ELSE 1 END ASC",
        SortDirection::Desc => "CASE WHEN completed_downloads.asin IS NULL THEN 0 ELSE 1 END DESC",
    }
}

fn grouped_order_expression(field: SortField, direction: SortDirection) -> &'static str {
    match (field, direction) {
        (SortField::Title, SortDirection::Asc) => "b.title ASC",
        (SortField::Title, SortDirection::Desc) => "b.title DESC",
        (SortField::ReleaseDate, SortDirection::Asc) => "b.date_published ASC, b.title ASC",
        (SortField::ReleaseDate, SortDirection::Desc) => "b.date_published DESC, b.title ASC",
        (SortField::DateAdded, SortDirection::Asc) => "COALESCE(book_accounts.date_added, lb.date_added) ASC, b.title ASC",
        (SortField::DateAdded, SortDirection::Desc) => "COALESCE(book_accounts.date_added, lb.date_added) DESC, b.title ASC",
        (SortField::Length, SortDirection::Asc) => "b.length_in_minutes ASC, b.title ASC",
        (SortField::Length, SortDirection::Desc) => "b.length_in_minutes DESC, b.title ASC",
        (SortField::Series, SortDirection::Asc) => {
            "CASE WHEN book_series_first.series_name IS NULL THEN 1 ELSE 0 END, book_series_first.series_name ASC, book_series_first.series_sequence ASC, b.title ASC"
        },
        (SortField::Series, SortDirection::Desc) => {
            "CASE WHEN book_series_first.series_name IS NULL THEN 1 ELSE 0 END, book_series_first.series_name DESC, book_series_first.series_sequence DESC, b.title ASC"
        },
        (SortField::Downloaded, _) => "b.title ASC",
    }
}

/// Filter and search parameters for book queries
#[derive(Debug, Clone, Default)]
pub struct BookQueryParams {
    pub search_query: Option<String>, // Search in title, author, narrator
    pub series_names: Vec<String>,    // Filter by series (any match)
    pub categories: Vec<String>,      // Filter by genre/category (any match)
    pub source: Option<String>,       // Filter by source (audible, librivox)
    pub accounts: Vec<String>,        // Filter by owning accounts (any match)
    pub origin_asin: Option<String>,  // Filter by podcast/periodical parent ASIN
    pub include_podcasts: bool,
    pub podcasts_only: bool, // Only podcast/periodical parents
    pub sort_field: Option<SortField>,
    pub sort_direction: Option<SortDirection>,
    pub downloaded_group_sort_field: Option<SortField>,
    pub downloaded_group_sort_direction: Option<SortDirection>,
    pub limit: i64,
    pub offset: i64,
}

impl BookQueryParams {
    pub fn with_defaults() -> Self {
        Self {
            include_podcasts: true,
            ..Default::default()
        }
    }
}

/// Build the shared WHERE clause and bind values for book filter queries.
/// Used by both the list and count queries so they always agree.
fn build_book_filter_where(params: &BookQueryParams) -> (String, Vec<String>) {
    let mut where_clauses: Vec<String> = Vec::new();
    let mut bind_values: Vec<String> = Vec::new();

    // Search filter
    if let Some(ref search) = params.search_query {
        let pattern = format!("%{}%", search);
        where_clauses.push(
            "(b.title LIKE ? OR b.subtitle LIKE ? OR book_authors.authors LIKE ? \
             OR book_narrators.narrators LIKE ? OR book_series_first.series_name LIKE ?)"
                .to_string(),
        );
        for _ in 0..5 {
            bind_values.push(pattern.clone());
        }
    }

    // Series filter (any of the selected series; matches every series a book
    // belongs to, not just the first one)
    let series: Vec<&String> = params
        .series_names
        .iter()
        .filter(|s| !s.trim().is_empty())
        .collect();
    if !series.is_empty() {
        let placeholders = vec!["?"; series.len()].join(", ");
        where_clauses.push(format!(
            "EXISTS (SELECT 1 FROM SeriesBooks sb_f \
             JOIN Series s_f ON sb_f.series_id = s_f.series_id \
             WHERE sb_f.book_id = b.book_id AND s_f.name IN ({}))",
            placeholders
        ));
        for s in series {
            bind_values.push(s.clone());
        }
    }

    // Category filter (any of the selected categories)
    let categories: Vec<&String> = params
        .categories
        .iter()
        .filter(|c| !c.trim().is_empty())
        .collect();
    if !categories.is_empty() {
        let likes = vec!["cl.ladder LIKE ?"; categories.len()].join(" OR ");
        where_clauses.push(format!(
            "EXISTS (SELECT 1 FROM BookCategories bc \
             JOIN CategoryLadders cl ON bc.category_ladder_id = cl.category_ladder_id \
             WHERE bc.book_id = b.book_id AND ({}))",
            likes
        ));
        for c in categories {
            // Ladders store a JSON array of names; quote for an exact match.
            bind_values.push(format!("%\"{}\"%", c));
        }
    }

    // Source filter
    if let Some(ref source) = params.source {
        where_clauses.push("COALESCE(b.source, 'audible') = ?".to_string());
        bind_values.push(source.clone());
    }

    // Account filter (any of the selected accounts)
    let accounts: Vec<&String> = params
        .accounts
        .iter()
        .filter(|a| !a.trim().is_empty())
        .collect();
    if !accounts.is_empty() {
        let placeholders = vec!["?"; accounts.len()].join(", ");
        where_clauses.push(format!(
            "EXISTS (SELECT 1 FROM BookAccounts ba_filter \
             WHERE ba_filter.book_id = b.book_id AND ba_filter.account IN ({}) \
             AND ba_filter.is_deleted = 0)",
            placeholders
        ));
        for a in accounts {
            bind_values.push(a.clone());
        }
    }

    let mut has_origin_filter = false;
    if let Some(ref origin_asin) = params.origin_asin {
        if !origin_asin.trim().is_empty() {
            has_origin_filter = true;
            where_clauses.push("b.origin_asin = ?".to_string());
            bind_values.push(origin_asin.clone());
            where_clauses.push("b.audible_product_id != ?".to_string());
            bind_values.push(origin_asin.clone());
        }
    }

    if !has_origin_filter {
        where_clauses.push("b.content_type != 2".to_string());
    }

    if params.podcasts_only {
        where_clauses.push(
            "b.content_delivery_type IN ('PodcastParent', 'PodcastSeries', 'Periodical')"
                .to_string(),
        );
    } else if !params.include_podcasts {
        where_clauses.push(
            "(b.content_delivery_type IS NULL \
             OR b.content_delivery_type NOT IN ('PodcastParent', 'PodcastSeries', 'Periodical'))"
                .to_string(),
        );
    }

    let where_clause = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_clauses.join(" AND "))
    };

    (where_clause, bind_values)
}

/// List books with relations, supporting search, filter, and sort
pub async fn list_books_with_filters(
    pool: &SqlitePool,
    params: &BookQueryParams,
) -> Result<Vec<BookWithRelations>> {
    let (where_clause, bind_values) = build_book_filter_where(params);

    // Build ORDER BY clause
    let order_clause = match (params.sort_field, params.sort_direction) {
        (Some(SortField::Title), Some(SortDirection::Asc)) => "ORDER BY b.title ASC".to_string(),
        (Some(SortField::Title), Some(SortDirection::Desc)) => "ORDER BY b.title DESC".to_string(),
        (Some(SortField::ReleaseDate), Some(SortDirection::Asc)) => "ORDER BY b.date_published ASC".to_string(),
        (Some(SortField::ReleaseDate), Some(SortDirection::Desc)) => "ORDER BY b.date_published DESC".to_string(),
        (Some(SortField::DateAdded), Some(SortDirection::Asc)) => "ORDER BY COALESCE(book_accounts.date_added, lb.date_added) ASC".to_string(),
        (Some(SortField::DateAdded), Some(SortDirection::Desc)) => "ORDER BY COALESCE(book_accounts.date_added, lb.date_added) DESC".to_string(),
        (Some(SortField::Length), Some(SortDirection::Asc)) => "ORDER BY b.length_in_minutes ASC, b.title ASC".to_string(),
        (Some(SortField::Length), Some(SortDirection::Desc)) => "ORDER BY b.length_in_minutes DESC, b.title ASC".to_string(),
        (Some(SortField::Downloaded), direction) => {
            let group_direction = direction.unwrap_or(SortDirection::Desc);
            let field_direction = params
                .downloaded_group_sort_direction
                .unwrap_or(SortDirection::Asc);
            let field = params.downloaded_group_sort_field.unwrap_or(SortField::Title);

            format!(
                "ORDER BY {}, {}",
                downloaded_status_order_expression(group_direction),
                grouped_order_expression(field, field_direction)
            )
        },
        (Some(SortField::Series), Some(SortDirection::Asc)) => {
            "ORDER BY CASE WHEN book_series_first.series_name IS NULL THEN 1 ELSE 0 END, book_series_first.series_name ASC, book_series_first.series_sequence ASC".to_string()
        },
        (Some(SortField::Series), Some(SortDirection::Desc)) => {
            "ORDER BY CASE WHEN book_series_first.series_name IS NULL THEN 1 ELSE 0 END, book_series_first.series_name DESC, book_series_first.series_sequence DESC".to_string()
        },
        _ => "ORDER BY b.title ASC".to_string(), // Default
    };

    // Build complete query
    let query = format!(
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
            LIMIT 1
        ),
        book_series AS (
            SELECT
                sb.book_id,
                s.name as series_name,
                sb."index" as series_sequence,
                ROW_NUMBER() OVER (PARTITION BY sb.book_id ORDER BY sb."index", s.name) as rn
            FROM SeriesBooks sb
            JOIN Series s ON sb.series_id = s.series_id
        ),
        book_series_first AS (
            SELECT book_id, series_name, series_sequence
            FROM book_series
            WHERE rn = 1
        ),
        completed_downloads AS (
            SELECT asin, MAX(completed_at) as completed_at
            FROM DownloadTasks
            WHERE status = 'completed' AND output_path != ''
            GROUP BY asin
        ),
        book_accounts AS (
            SELECT
                book_id,
                GROUP_CONCAT(account, ',') as accounts,
                MIN(date_added) as date_added
            FROM BookAccounts
            WHERE is_deleted = 0
            GROUP BY book_id
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
            COALESCE(b.source, 'audible') as source,
            book_authors.authors as authors_str,
            book_narrators.narrators as narrators_str,
            book_publishers.publisher,
            book_series_first.series_name,
            book_series_first.series_sequence,
            COALESCE(book_accounts.date_added, lb.date_added) as purchase_date,
            book_accounts.accounts as account
        FROM Books b
        LEFT JOIN LibraryBooks lb ON b.book_id = lb.book_id
        LEFT JOIN book_accounts ON b.book_id = book_accounts.book_id
        LEFT JOIN book_authors ON b.book_id = book_authors.book_id
        LEFT JOIN book_narrators ON b.book_id = book_narrators.book_id
        LEFT JOIN book_publishers ON b.book_id = book_publishers.book_id
        LEFT JOIN book_series_first ON b.book_id = book_series_first.book_id
        LEFT JOIN completed_downloads ON completed_downloads.asin = b.audible_product_id
        {}
        {}
        LIMIT ? OFFSET ?
        "#,
        where_clause, order_clause
    );

    // Build query with bindings
    let mut q = sqlx::query_as::<_, BookWithRelations>(&query);

    for value in bind_values {
        q = q.bind(value);
    }

    q = q.bind(params.limit).bind(params.offset);

    let books = q.fetch_all(pool).await?;

    Ok(books)
}

/// Count books matching filter criteria
pub async fn count_books_with_filters(pool: &SqlitePool, params: &BookQueryParams) -> Result<i64> {
    let (where_clause, bind_values) = build_book_filter_where(params);

    let query = format!(
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
        book_series AS (
            SELECT
                sb.book_id,
                s.name as series_name,
                ROW_NUMBER() OVER (PARTITION BY sb.book_id ORDER BY sb."index", s.name) as rn
            FROM SeriesBooks sb
            JOIN Series s ON sb.series_id = s.series_id
        ),
        book_series_first AS (
            SELECT book_id, series_name
            FROM book_series
            WHERE rn = 1
        )
        SELECT COUNT(DISTINCT b.book_id)
        FROM Books b
        LEFT JOIN LibraryBooks lb ON b.book_id = lb.book_id
        LEFT JOIN book_authors ON b.book_id = book_authors.book_id
        LEFT JOIN book_narrators ON b.book_id = book_narrators.book_id
        LEFT JOIN book_series_first ON b.book_id = book_series_first.book_id
        {}
        "#,
        where_clause
    );

    let mut q = sqlx::query_scalar::<_, i64>(&query);

    for value in bind_values {
        q = q.bind(value);
    }

    let count = q.fetch_one(pool).await?;

    Ok(count)
}

/// Get all unique series names from the library
pub async fn list_all_series(pool: &SqlitePool) -> Result<Vec<String>> {
    let series: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT s.name FROM Series s \
         JOIN SeriesBooks sb ON s.series_id = sb.series_id \
         ORDER BY s.name",
    )
    .fetch_all(pool)
    .await?;

    Ok(series)
}

/// Get all unique categories/genres from the library
///
/// CategoryLadders.ladder stores a JSON array of category names for one
/// ladder (e.g. ["Science Fiction & Fantasy", "Science Fiction"]); every
/// name on a ladder linked to at least one book becomes a filter option.
pub async fn list_all_categories(pool: &SqlitePool) -> Result<Vec<String>> {
    let categories: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT je.value FROM CategoryLadders cl \
         JOIN BookCategories bc ON cl.category_ladder_id = bc.category_ladder_id, \
         json_each(cl.ladder) je \
         WHERE je.value IS NOT NULL AND je.value != '' \
         ORDER BY je.value",
    )
    .fetch_all(pool)
    .await?;

    Ok(categories)
}

/// Search books by title
pub async fn search_books_by_title(
    pool: &SqlitePool,
    query: &str,
    limit: i64,
) -> Result<Vec<Book>> {
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

    sqlx::query(
        r#"
        INSERT INTO BookAccounts (book_id, account)
        VALUES (?, ?)
        ON CONFLICT(book_id, account) DO UPDATE SET
            is_deleted = 0,
            absent_from_last_scan = 0
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
pub async fn list_library_books_by_account(
    pool: &SqlitePool,
    account: &str,
) -> Result<Vec<LibraryBook>> {
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
pub async fn find_user_defined_item(
    pool: &SqlitePool,
    book_id: i64,
) -> Result<Option<UserDefinedItem>> {
    let item =
        sqlx::query_as::<_, UserDefinedItem>("SELECT * FROM UserDefinedItems WHERE book_id = ?")
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
    let result =
        sqlx::query("INSERT INTO Contributors (name, audible_contributor_id) VALUES (?, ?)")
            .bind(&contributor.name)
            .bind(&contributor.audible_contributor_id)
            .execute(pool)
            .await?;

    Ok(result.last_insert_rowid())
}

/// Find contributors by book ID and role
pub async fn find_contributors_by_book(
    pool: &SqlitePool,
    book_id: i64,
    role: i32,
) -> Result<Vec<Contributor>> {
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
pub async fn remove_book_contributors_by_role(
    pool: &SqlitePool,
    book_id: i64,
    role: i32,
) -> Result<()> {
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
    let existing: Option<i64> =
        sqlx::query_scalar("SELECT series_id FROM Series WHERE audible_series_id = ?")
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
    let result = sqlx::query("INSERT INTO Series (audible_series_id, name) VALUES (?, ?)")
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
pub async fn find_series_by_book(
    pool: &SqlitePool,
    book_id: i64,
) -> Result<Vec<(Series, SeriesBook)>> {
    let results =
        sqlx::query_as::<_, (i64, String, Option<String>, i64, i64, Option<String>, f32)>(
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
        .map(
            |(series_id, audible_series_id, name, sb_series_id, sb_book_id, order, index)| {
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
            },
        )
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
        let existing: Option<i64> =
            sqlx::query_scalar("SELECT category_id FROM Categories WHERE audible_category_id = ?")
                .bind(audible_id)
                .fetch_optional(pool)
                .await?;

        if let Some(id) = existing {
            return Ok(id);
        }
    }

    // Insert new category
    let result = sqlx::query("INSERT INTO Categories (audible_category_id, name) VALUES (?, ?)")
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
    let result =
        sqlx::query("INSERT INTO CategoryLadders (audible_ladder_id, ladder) VALUES (?, ?)")
            .bind(&ladder.audible_ladder_id)
            .bind(&ladder.ladder)
            .execute(pool)
            .await?;

    Ok(result.last_insert_rowid())
}

/// Link book to category ladder
pub async fn add_book_category(
    pool: &SqlitePool,
    book_id: i64,
    category_ladder_id: i64,
) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO BookCategories (book_id, category_ladder_id) VALUES (?, ?)")
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
    let result = sqlx::query("INSERT INTO Supplements (book_id, url) VALUES (?, ?)")
        .bind(book_id)
        .bind(url)
        .execute(pool)
        .await?;

    Ok(result.last_insert_rowid())
}

/// Find supplements for a book
pub async fn find_supplements_by_book(pool: &SqlitePool, book_id: i64) -> Result<Vec<Supplement>> {
    let supplements =
        sqlx::query_as::<_, Supplement>("SELECT * FROM Supplements WHERE book_id = ?")
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

/// Clear all library data (for testing)
///
/// Deletes all books and related data from the database.
/// Use with caution - this is irreversible!
/// Clear download state for all books
///
/// Resets all download-related fields in UserDefinedItems table:
/// - book_status -> 0 (NotLiberated)
/// - pdf_status -> NULL
/// - last_downloaded -> NULL
/// - last_downloaded_version -> NULL
/// - last_downloaded_format -> NULL
/// - last_downloaded_file_version -> NULL
///
/// This keeps all book metadata but resets download status, useful for testing
/// or when re-downloading the entire library.
pub async fn clear_download_state(pool: &SqlitePool) -> Result<i64> {
    let result = sqlx::query(
        r#"
        UPDATE UserDefinedItems
        SET book_status = 0,
            pdf_status = NULL,
            last_downloaded = NULL,
            last_downloaded_version = NULL,
            last_downloaded_format = NULL,
            last_downloaded_file_version = NULL
        "#,
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected() as i64)
}

/// Clear download state for a single book by ASIN.
///
/// This resets the download status for a specific book, clearing:
/// - book_status -> 0 (NotLiberated)
/// - pdf_status -> NULL
/// - last_downloaded -> NULL
/// - last_downloaded_version -> NULL
/// - last_downloaded_format -> NULL
/// - last_downloaded_file_version -> NULL
///
/// Also deletes any download tasks for this book to reset to default state.
/// Optionally deletes the downloaded file from disk.
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `asin` - Audible product ID (ASIN)
/// * `delete_file` - If true, also delete the downloaded file
///
/// # Returns
/// * `Ok(file_path)` - Returns the file path if it existed and was deleted, None otherwise
/// * `Err` if book not found or database error
pub async fn clear_book_download_state(
    pool: &SqlitePool,
    asin: &str,
    delete_file: bool,
) -> Result<Option<String>> {
    // First verify the book exists
    let book = find_book_by_asin(pool, asin).await?;
    if book.is_none() {
        return Err(crate::LibationError::InvalidInput(format!(
            "Book with ASIN {} not found",
            asin
        )));
    }

    let book_id = book.unwrap().book_id;

    // Get the output path from completed download tasks
    let file_path: Option<String> = sqlx::query_scalar(
        r#"
        SELECT output_path
        FROM DownloadTasks
        WHERE asin = ? AND status = 'completed'
        ORDER BY completed_at DESC
        LIMIT 1
        "#,
    )
    .bind(asin)
    .fetch_optional(pool)
    .await?;

    // Delete the file if requested and file path exists
    let deleted_path = if delete_file {
        if let Some(ref path) = file_path {
            match tokio::fs::remove_file(path).await {
                Ok(_) => {
                    println!("[clear_book_download_state] Deleted file: {}", path);
                    Some(path.clone())
                }
                Err(e) => {
                    eprintln!(
                        "[clear_book_download_state] Failed to delete file {}: {}",
                        path, e
                    );
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    // Clear download state in UserDefinedItems
    sqlx::query(
        r#"
        UPDATE UserDefinedItems
        SET book_status = 0,
            pdf_status = NULL,
            last_downloaded = NULL,
            last_downloaded_version = NULL,
            last_downloaded_format = NULL,
            last_downloaded_file_version = NULL
        WHERE book_id = ?
        "#,
    )
    .bind(book_id)
    .execute(pool)
    .await?;

    // Delete any download tasks for this book to reset to default state
    sqlx::query(
        r#"
        DELETE FROM DownloadTasks
        WHERE asin = ?
        "#,
    )
    .bind(asin)
    .execute(pool)
    .await?;

    Ok(deleted_path)
}

/// Get the downloaded file path for a book by ASIN.
///
/// Returns the output path from the most recent completed download task.
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `asin` - Audible product ID (ASIN)
///
/// # Returns
/// * `Ok(Some(path))` if a completed download exists
/// * `Ok(None)` if no completed download found
pub async fn get_book_file_path(pool: &SqlitePool, asin: &str) -> Result<Option<String>> {
    let file_path: Option<String> = sqlx::query_scalar(
        r#"
        SELECT output_path
        FROM DownloadTasks
        WHERE asin = ? AND status = 'completed'
        ORDER BY completed_at DESC
        LIMIT 1
        "#,
    )
    .bind(asin)
    .fetch_optional(pool)
    .await?;

    Ok(file_path)
}

/// Get latest completed download file paths keyed by ASIN.
pub async fn get_completed_download_paths(pool: &SqlitePool) -> Result<HashMap<String, String>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        r#"
        SELECT dt.asin, dt.output_path
        FROM DownloadTasks dt
        INNER JOIN (
            SELECT asin, MAX(completed_at) AS completed_at
            FROM DownloadTasks
            WHERE status = 'completed' AND output_path != ''
            GROUP BY asin
        ) latest
            ON latest.asin = dt.asin
            AND latest.completed_at = dt.completed_at
        WHERE dt.status = 'completed' AND dt.output_path != ''
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().collect())
}

/// Set the file path for a book by creating a manually completed download task.
///
/// This allows users to mark a book as downloaded by associating it with an
/// existing audio file on disk. Creates or updates a completed download task.
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `asin` - Audible product ID (ASIN)
/// * `title` - Book title
/// * `file_path` - Absolute path to the audio file
///
/// # Returns
/// * `Ok(task_id)` - ID of the created download task
pub async fn set_book_file_path(
    pool: &SqlitePool,
    asin: &str,
    title: &str,
    file_path: &str,
) -> Result<String> {
    use uuid::Uuid;
    let now = chrono::Utc::now().to_rfc3339();

    let existing_task_id: Option<String> = sqlx::query_scalar(
        r#"
        SELECT task_id
        FROM DownloadTasks
        WHERE asin = ? AND status = 'completed'
        ORDER BY completed_at DESC
        LIMIT 1
        "#,
    )
    .bind(asin)
    .fetch_optional(pool)
    .await?;

    if let Some(task_id) = existing_task_id {
        sqlx::query(
            r#"
            UPDATE DownloadTasks
            SET title = ?, output_path = ?, completed_at = ?
            WHERE task_id = ?
            "#,
        )
        .bind(title)
        .bind(file_path)
        .bind(&now)
        .bind(&task_id)
        .execute(pool)
        .await?;

        return Ok(task_id);
    }

    let task_id = Uuid::new_v4().to_string();

    sqlx::query(
        r#"
        INSERT INTO DownloadTasks (
            task_id, asin, title, status, bytes_downloaded, total_bytes,
            download_url, download_path, output_path, request_headers,
            retry_count, created_at, started_at, completed_at
        )
        VALUES (?, ?, ?, 'completed', 0, 0, '', '', ?, '{}', 0, ?, ?, ?)
        "#,
    )
    .bind(&task_id)
    .bind(asin)
    .bind(title)
    .bind(file_path)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    Ok(task_id)
}

pub async fn clear_library(pool: &SqlitePool) -> Result<()> {
    // Delete in correct order to respect foreign keys
    sqlx::query("DELETE FROM LibraryBooks")
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM SeriesBooks").execute(pool).await?;
    sqlx::query("DELETE FROM BookContributors")
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM BookCategories")
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM UserDefinedItems")
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM Supplements").execute(pool).await?;
    sqlx::query("DELETE FROM Books").execute(pool).await?;
    sqlx::query("DELETE FROM Series").execute(pool).await?;
    sqlx::query("DELETE FROM Contributors")
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM Categories").execute(pool).await?;
    sqlx::query("DELETE FROM CategoryLadders")
        .execute(pool)
        .await?;

    Ok(())
}

/// Insert a LibriVox book with all related data in one operation.
/// Insert a Libro.fm book (source='librofm', DRM-free). Product id = the ISBN.
/// Owned by [account] (the Libro.fm email). Skips if it already exists.
pub async fn insert_libro_book(
    pool: &SqlitePool,
    isbn: &str,
    title: &str,
    authors: &[String],
    narrators: &[String],
    description: &str,
    length_in_minutes: i32,
    cover_url: Option<&str>,
    account: &str,
) -> Result<i64> {
    if let Some(existing) = find_book_by_asin(pool, isbn).await? {
        return Ok(existing.book_id);
    }

    let result = sqlx::query(
        r#"
        INSERT INTO Books (
            audible_product_id, title, subtitle, description, length_in_minutes,
            content_type, locale, picture_large, source,
            is_abridged, is_spatial, language,
            rating_overall, rating_performance, rating_story,
            is_finished, is_downloadable, is_ayce
        ) VALUES (?, ?, NULL, ?, ?, 1, ?, ?, 'librofm', 0, 0, ?, 0.0, 0.0, 0.0, 0, 1, 0)
        "#,
    )
    .bind(isbn)
    .bind(title)
    .bind(description)
    .bind(length_in_minutes)
    .bind("us")
    .bind(cover_url)
    .bind("english")
    .execute(pool)
    .await?;

    let book_id = result.last_insert_rowid();

    insert_library_book(
        pool,
        &NewLibraryBook {
            book_id,
            account: account.to_string(),
        },
    )
    .await?;
    insert_user_defined_item(pool, &NewUserDefinedItem::new(book_id)).await?;

    for (i, author) in authors.iter().enumerate() {
        let contributor_id = upsert_contributor(pool, &NewContributor::new(author.clone())).await?;
        add_book_contributor(pool, book_id, contributor_id, Role::Author as i32, i as i16).await?;
    }
    for (i, narrator) in narrators.iter().enumerate() {
        let contributor_id =
            upsert_contributor(pool, &NewContributor::new(narrator.clone())).await?;
        add_book_contributor(pool, book_id, contributor_id, Role::Narrator as i32, i as i16)
            .await?;
    }

    Ok(book_id)
}

///
/// This handles inserting into Books (with source='librivox'), LibraryBooks,
/// UserDefinedItems, and Contributors tables.
pub async fn insert_librivox_book(
    pool: &SqlitePool,
    librivox_id: &str,
    title: &str,
    authors: &[String],
    narrators: &[String],
    description: &str,
    length_in_minutes: i32,
    language: &str,
    cover_url: Option<&str>,
) -> Result<i64> {
    let product_id = format!("librivox_{}", librivox_id);

    // Check if already exists
    if let Some(existing) = find_book_by_asin(pool, &product_id).await? {
        return Ok(existing.book_id);
    }

    // Insert book with source='librivox'
    let result = sqlx::query(
        r#"
        INSERT INTO Books (
            audible_product_id, title, subtitle, description, length_in_minutes,
            content_type, locale, picture_large, source,
            is_abridged, is_spatial, language,
            rating_overall, rating_performance, rating_story,
            is_finished, is_downloadable, is_ayce
        ) VALUES (?, ?, NULL, ?, ?, 1, ?, ?, 'librivox', 0, 0, ?, 0.0, 0.0, 0.0, 0, 1, 0)
        "#,
    )
    .bind(&product_id)
    .bind(title)
    .bind(description)
    .bind(length_in_minutes)
    .bind(language)
    .bind(cover_url)
    .bind(language)
    .execute(pool)
    .await?;

    let book_id = result.last_insert_rowid();

    // Insert LibraryBook
    let lib_book = NewLibraryBook {
        book_id,
        account: "librivox".to_string(),
    };
    insert_library_book(pool, &lib_book).await?;

    // Insert UserDefinedItem
    insert_user_defined_item(pool, &NewUserDefinedItem::new(book_id)).await?;

    // Insert authors
    for (i, author) in authors.iter().enumerate() {
        let contributor = NewContributor::new(author.clone());
        let contributor_id = upsert_contributor(pool, &contributor).await?;
        add_book_contributor(pool, book_id, contributor_id, Role::Author as i32, i as i16).await?;
    }

    // Insert narrators
    for (i, narrator) in narrators.iter().enumerate() {
        let contributor = NewContributor::new(narrator.clone());
        let contributor_id = upsert_contributor(pool, &contributor).await?;
        add_book_contributor(
            pool,
            book_id,
            contributor_id,
            Role::Narrator as i32,
            i as i16,
        )
        .await?;
    }

    Ok(book_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::database::Database;

    #[tokio::test]
    async fn test_insert_and_find_book() {
        let db = Database::new_in_memory()
            .await
            .expect("Failed to create database");

        let new_book = NewBook::new(
            "B012345678".to_string(),
            "Test Book".to_string(),
            "us".to_string(),
        );

        let book_id = insert_book(db.pool(), &new_book)
            .await
            .expect("Failed to insert book");
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
        let db = Database::new_in_memory()
            .await
            .expect("Failed to create database");

        let new_book = NewBook::new(
            "B012345679".to_string(),
            "Test Book Original".to_string(),
            "us".to_string(),
        );

        // First upsert - should insert
        let book_id1 = upsert_book(db.pool(), &new_book)
            .await
            .expect("Failed to upsert book");

        // Second upsert with same ASIN - should update
        let mut updated_book = new_book.clone();
        updated_book.title = "Test Book Updated".to_string();
        let book_id2 = upsert_book(db.pool(), &updated_book)
            .await
            .expect("Failed to upsert book");

        assert_eq!(book_id1, book_id2, "Book ID should be the same on update");

        let found = find_book_by_id(db.pool(), book_id1)
            .await
            .expect("Failed to find book");
        assert_eq!(found.unwrap().title, "Test Book Updated");
    }

    #[tokio::test]
    async fn test_contributor_operations() {
        let db = Database::new_in_memory()
            .await
            .expect("Failed to create database");

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

    #[tokio::test]
    async fn test_set_book_file_path_updates_existing_completed_task() {
        let db = Database::new_in_memory()
            .await
            .expect("Failed to create database");

        let task_id = set_book_file_path(db.pool(), "B012345680", "Test Book", "/books/old.m4b")
            .await
            .expect("Failed to set file path");

        let updated_task_id =
            set_book_file_path(db.pool(), "B012345680", "Test Book", "/books/new.m4b")
                .await
                .expect("Failed to update file path");

        assert_eq!(task_id, updated_task_id);

        let task_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM DownloadTasks WHERE asin = ?")
                .bind("B012345680")
                .fetch_one(db.pool())
                .await
                .expect("Failed to count download tasks");
        assert_eq!(task_count, 1);

        let file_path = get_book_file_path(db.pool(), "B012345680")
            .await
            .expect("Failed to get file path");
        assert_eq!(file_path.as_deref(), Some("/books/new.m4b"));

        let paths = get_completed_download_paths(db.pool())
            .await
            .expect("Failed to get completed paths");
        assert_eq!(
            paths.get("B012345680").map(String::as_str),
            Some("/books/new.m4b")
        );
    }

    #[tokio::test]
    async fn test_list_books_with_filters_by_series_and_category() {
        let db = Database::new_in_memory()
            .await
            .expect("Failed to create database");

        let books = [
            ("B000000101", "Book One", Some("Alpha Saga"), Some("Fantasy")),
            ("B000000102", "Book Two", Some("Beta Cycle"), Some("Sci-Fi")),
            ("B000000103", "Book Three", None, None),
        ];

        for (asin, title, series, category) in books {
            let book = NewBook::new(asin.to_string(), title.to_string(), "us".to_string());
            let book_id = insert_book(db.pool(), &book)
                .await
                .expect("Failed to insert book");

            insert_library_book(
                db.pool(),
                &NewLibraryBook {
                    book_id,
                    account: "test@example.com".to_string(),
                },
            )
            .await
            .expect("Failed to insert library book");

            if let Some(series_name) = series {
                let series_id = upsert_series(
                    db.pool(),
                    &NewSeries {
                        audible_series_id: format!("SER-{}", series_name),
                        name: Some(series_name.to_string()),
                    },
                )
                .await
                .expect("Failed to upsert series");
                add_book_to_series(db.pool(), series_id, book_id, Some("1".to_string()), 1.0)
                    .await
                    .expect("Failed to link series");
            }

            if let Some(category_name) = category {
                let ladder_id = upsert_category_ladder(
                    db.pool(),
                    &NewCategoryLadder {
                        audible_ladder_id: format!("LAD-{}", category_name),
                        ladder: format!("[\"{}\"]", category_name),
                    },
                )
                .await
                .expect("Failed to upsert ladder");
                add_book_category(db.pool(), book_id, ladder_id)
                    .await
                    .expect("Failed to link category");
            }
        }

        // Series-only filter: this returned zero rows before the fix because
        // the list query referenced a CTE it never joined.
        let params = BookQueryParams {
            series_names: vec!["Alpha Saga".to_string()],
            limit: 10,
            offset: 0,
            ..BookQueryParams::with_defaults()
        };
        let by_series = list_books_with_filters(db.pool(), &params)
            .await
            .expect("Failed to list by series");
        assert_eq!(by_series.len(), 1);
        assert_eq!(by_series[0].title, "Book One");
        assert_eq!(
            count_books_with_filters(db.pool(), &params)
                .await
                .expect("Failed to count by series"),
            1
        );

        // Multi-select series
        let params = BookQueryParams {
            series_names: vec!["Alpha Saga".to_string(), "Beta Cycle".to_string()],
            limit: 10,
            offset: 0,
            ..BookQueryParams::with_defaults()
        };
        assert_eq!(
            list_books_with_filters(db.pool(), &params)
                .await
                .expect("Failed to list by two series")
                .len(),
            2
        );

        // Category filter against JSON name ladders
        let params = BookQueryParams {
            categories: vec!["Fantasy".to_string()],
            limit: 10,
            offset: 0,
            ..BookQueryParams::with_defaults()
        };
        let by_category = list_books_with_filters(db.pool(), &params)
            .await
            .expect("Failed to list by category");
        assert_eq!(by_category.len(), 1);
        assert_eq!(by_category[0].title, "Book One");

        // Genre option list comes from the JSON ladders
        let categories = list_all_categories(db.pool())
            .await
            .expect("Failed to list categories");
        assert_eq!(categories, vec!["Fantasy".to_string(), "Sci-Fi".to_string()]);
    }

    #[tokio::test]
    async fn test_list_books_with_filters_sorts_by_length() {
        let db = Database::new_in_memory()
            .await
            .expect("Failed to create database");

        let books = [
            ("B000000001", "Medium Book", 120),
            ("B000000002", "Short Book", 45),
            ("B000000003", "Long Book", 360),
        ];

        for (asin, title, length_in_minutes) in books {
            let mut book = NewBook::new(asin.to_string(), title.to_string(), "us".to_string());
            book.length_in_minutes = length_in_minutes;

            let book_id = insert_book(db.pool(), &book)
                .await
                .expect("Failed to insert book");

            insert_library_book(
                db.pool(),
                &NewLibraryBook {
                    book_id,
                    account: "test@example.com".to_string(),
                },
            )
            .await
            .expect("Failed to insert library book");
        }

        let params = BookQueryParams {
            sort_field: Some(SortField::Length),
            sort_direction: Some(SortDirection::Asc),
            limit: 10,
            offset: 0,
            ..BookQueryParams::with_defaults()
        };

        let ascending = list_books_with_filters(db.pool(), &params)
            .await
            .expect("Failed to list books");

        assert_eq!(
            ascending
                .iter()
                .map(|book| book.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Short Book", "Medium Book", "Long Book"]
        );

        let params = BookQueryParams {
            sort_direction: Some(SortDirection::Desc),
            ..params
        };

        let descending = list_books_with_filters(db.pool(), &params)
            .await
            .expect("Failed to list books");

        assert_eq!(
            descending
                .iter()
                .map(|book| book.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Long Book", "Medium Book", "Short Book"]
        );
    }

    #[tokio::test]
    async fn test_list_books_with_filters_sorts_by_downloaded_status() {
        let db = Database::new_in_memory()
            .await
            .expect("Failed to create database");

        let books = [
            ("B000000001", "Missing Long", 300),
            ("B000000002", "Downloaded Short", 30),
            ("B000000003", "Missing Short", 60),
            ("B000000004", "Downloaded Long", 240),
        ];

        for (asin, title, length_in_minutes) in books {
            let mut book = NewBook::new(asin.to_string(), title.to_string(), "us".to_string());
            book.length_in_minutes = length_in_minutes;

            let book_id = insert_book(db.pool(), &book)
                .await
                .expect("Failed to insert book");

            insert_library_book(
                db.pool(),
                &NewLibraryBook {
                    book_id,
                    account: "test@example.com".to_string(),
                },
            )
            .await
            .expect("Failed to insert library book");
        }

        set_book_file_path(
            db.pool(),
            "B000000002",
            "Downloaded Short",
            "/books/downloaded-short.m4b",
        )
        .await
        .expect("Failed to mark book downloaded");

        set_book_file_path(
            db.pool(),
            "B000000004",
            "Downloaded Long",
            "/books/downloaded-long.m4b",
        )
        .await
        .expect("Failed to mark book downloaded");

        let params = BookQueryParams {
            sort_field: Some(SortField::Downloaded),
            sort_direction: Some(SortDirection::Desc),
            downloaded_group_sort_field: Some(SortField::Length),
            downloaded_group_sort_direction: Some(SortDirection::Asc),
            limit: 10,
            offset: 0,
            ..BookQueryParams::with_defaults()
        };

        let downloaded_first = list_books_with_filters(db.pool(), &params)
            .await
            .expect("Failed to list books");

        assert_eq!(
            downloaded_first
                .iter()
                .map(|book| book.title.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Downloaded Short",
                "Downloaded Long",
                "Missing Short",
                "Missing Long"
            ]
        );

        let params = BookQueryParams {
            sort_direction: Some(SortDirection::Asc),
            ..params
        };

        let downloaded_last = list_books_with_filters(db.pool(), &params)
            .await
            .expect("Failed to list books");

        assert_eq!(
            downloaded_last
                .iter()
                .map(|book| book.title.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Missing Short",
                "Missing Long",
                "Downloaded Short",
                "Downloaded Long"
            ]
        );
    }
}
