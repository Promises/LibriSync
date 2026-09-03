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

//! Library management and synchronization
//!
//! This module implements library sync functionality to retrieve and synchronize audiobook
//! data from the Audible API, porting from Libation's C# implementation.
//!
//! # Reference C# Sources
//! - **`AudibleUtilities/ApiExtended.cs`** - GetLibraryValidatedAsync() and getItemsAsync()
//! - **External: `AudibleApi/LibraryOptions.cs`** - Query parameters for library endpoint
//! - **External: `AudibleApi/Common/Item.cs`** - Library item model (LibraryDtoV10.cs)
//! - **`DtoImporterService/LibraryBookImporter.cs`** - Import library items to database
//! - **`DtoImporterService/BookImporter.cs`** - Import book metadata
//! - **`DtoImporterService/SeriesImporter.cs`** - Import series relationships
//! - **`DtoImporterService/ContributorImporter.cs`** - Import author/narrator data
//! - **`ApplicationServices/LibraryCommands.cs`** - High-level library sync operations
//!
//! # API Endpoint Reference
//! **Primary endpoint:** `GET https://api.audible.{domain}/1.0/library`
//!
//! **Query Parameters:**
//! - `num_results` - Page size (default 50, max 1000)
//! - `page` - Page number (starts at 1)
//! - `response_groups` - Comma-separated list of data groups to include:
//!   - `media` - Media metadata (formats, codecs)
//!   - `product_desc` - Product description
//!   - `product_extended_attrs` - Extended attributes
//!   - `relationships` - Series/episode relationships
//!   - `contributors` - Author/narrator details
//!   - `rating` - Rating information
//!   - `series` - Series information
//!   - `category_ladders` - Category hierarchies
//!   - `pdf_url` - PDF supplement URL
//!   - `origin_asin` - Original ASIN
//!   - `is_finished` - Completion status
//!   - `provided_review` - User review
//!   - `product_plans` - Subscription plans
//!
//! # Pagination Pattern (from ApiExtended.cs:98-123)
//! 1. Fetch pages concurrently (MaxConcurrency = 10)
//! 2. Process in batches of 50 items
//! 3. Handle episode/series parent relationships separately
//! 4. Merge all results into single collection
//!
//! # Database Upsert Strategy (from LibraryBookImporter.cs:30-96)
//! 1. Import books via BookImporter (creates/updates Book records)
//! 2. Upsert LibraryBook records (account ownership)
//! 3. Link contributors (authors, narrators, publishers)
//! 4. Link series with order
//! 5. Link categories via ladders
//! 6. Mark absent books (removed from library)

use crate::api::auth::Account;
use crate::api::client::AudibleClient;
use crate::error::{LibationError, Result};
use crate::storage::models::{
    Book, ContentType, LibraryBook, NewBook, NewCategory, NewCategoryLadder, NewContributor,
    NewLibraryBook, NewSeries, Role,
};
use crate::storage::Database;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

fn null_to_default<'de, D, T>(deserializer: D) -> std::result::Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

// ============================================================================
// API REQUEST/RESPONSE STRUCTURES
// ============================================================================

/// Library query options
/// Maps to C# `LibraryOptions` in AudibleApi/LibraryOptions.cs
///
/// Reference: ApiExtended.cs:122-133
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryOptions {
    /// Number of results per page (default 50, max 1000)
    #[serde(rename = "num_results")]
    pub number_of_results_per_page: i32,

    /// Page number (1-indexed)
    #[serde(rename = "page")]
    pub page_number: i32,

    /// Filter by purchase date (ISO 8601)
    #[serde(rename = "purchased_after", skip_serializing_if = "Option::is_none")]
    pub purchased_after: Option<String>,

    /// Response groups (controls which fields are included)
    /// Comma-separated string: "media,product_desc,relationships,contributors"
    #[serde(rename = "response_groups")]
    pub response_groups: String,

    /// Sort order (PURCHASE_DATE, TITLE, AUTHOR, etc.)
    #[serde(rename = "sort_by")]
    pub sort_by: String,

    /// Image sizes to include (e.g., "500,1215")
    #[serde(rename = "image_sizes", skip_serializing_if = "Option::is_none")]
    pub image_sizes: Option<String>,
}

impl Default for LibraryOptions {
    /// Default options for full library sync
    /// Reference: ApplicationServices/LibraryCommands.cs:122-133
    fn default() -> Self {
        Self {
            // Audible allows up to 1000 per page. Bigger pages mean fewer requests for a
            // large library (a 600-book library is 3 requests, not 13), and every request
            // is a chance for a transient empty/failed response to truncate the sync.
            number_of_results_per_page: 250,
            page_number: 1,
            purchased_after: None,
            response_groups: [
                "rating",
                "media",
                "relationships",
                "product_desc",
                "contributors",
                "provided_review",
                "product_plans",
                "series",
                "category_ladders",
                "product_extended_attrs",
                "pdf_url",
                "origin_asin",
                "is_finished",
            ]
            .join(","),
            sort_by: "PurchaseDate".to_string(),
            image_sizes: Some("500,1215".to_string()),
        }
    }
}

/// Library API response container
/// Maps to response from GET /1.0/library
#[derive(Debug, Clone, Deserialize)]
pub struct LibraryResponse {
    /// List of library items
    #[serde(default, deserialize_with = "null_to_default")]
    pub items: Vec<LibraryItem>,

    /// Total number of items in library (optional - not always included)
    #[serde(default)]
    pub total_results: Option<i32>,

    /// Current page number (optional - not always included)
    #[serde(default)]
    pub page: Option<i32>,

    /// Number of items in this page (optional - not always included)
    #[serde(default)]
    pub num_results: Option<i32>,

    /// Response groups included in response (array of strings)
    #[serde(default)]
    pub response_groups: Option<Vec<String>>,
}

/// Individual library item from Audible API
/// Maps to C# `Item` class in AudibleApi/Common/LibraryDtoV10.cs
///
/// This structure matches the JSON response from Audible's library endpoint.
/// Field names use snake_case to match Audible API JSON, with serde rename where needed.
#[derive(Debug, Clone, Deserialize)]
pub struct LibraryItem {
    // === CORE IDENTIFIERS ===
    /// Audible Standard Identification Number (unique product ID)
    pub asin: String,

    /// Primary title
    pub title: String,

    /// Subtitle (if present)
    #[serde(default)]
    pub subtitle: Option<String>,

    // === CONTENT TYPE ===
    /// Content type: "Product", "Episode", or "Parent"
    /// Maps to ContentType enum in database
    #[serde(default)]
    pub content_type: Option<String>,

    /// Content delivery type: "SinglePartBook", "MultiPartBook", etc.
    #[serde(default)]
    pub content_delivery_type: Option<String>,

    // === DATES ===
    /// Date added to library (purchase date)
    /// Optional on purpose: real accounts have titles with a missing or
    /// beginning-of-time `purchase_date`, and a book must not be lost over it.
    /// Reference: Libation 2c882e88 "#1378 : allow for invalid beginning-of-time
    /// 'purchase_date'. This is the case for an actual user".
    #[serde(rename = "purchase_date", default)]
    pub purchase_date: Option<DateTime<Utc>>,

    /// Release date (publication date)
    #[serde(rename = "release_date", default)]
    pub release_date: Option<NaiveDate>,

    /// Issue date (for serials/podcasts) - date only, no time
    #[serde(rename = "issue_date", default)]
    pub issue_date: Option<NaiveDate>,

    /// Publication date
    #[serde(rename = "publication_datetime", default)]
    pub publication_datetime: Option<DateTime<Utc>>,

    // === DESCRIPTION ===
    /// Short marketing teaser (often truncated with a trailing ellipsis)
    #[serde(rename = "merchandising_summary", default)]
    pub description: Option<String>,

    /// Full publisher summary from the product_desc response group
    #[serde(rename = "publisher_summary", default)]
    pub publisher_summary: Option<String>,

    /// Publisher/studio name
    #[serde(rename = "publisher_name", default)]
    pub publisher: Option<String>,

    // === AUDIO METADATA ===
    /// Runtime in minutes
    #[serde(rename = "runtime_length_min", default)]
    pub length_in_minutes: Option<i32>,

    /// Language code (e.g., "en_US")
    #[serde(default)]
    pub language: Option<String>,

    /// Is abridged version
    #[serde(rename = "is_abridged", default)]
    pub is_abridged: Option<bool>,

    /// Available audio codecs
    #[serde(
        rename = "available_codecs",
        default,
        deserialize_with = "null_to_default"
    )]
    pub available_codecs: Vec<CodecInfo>,

    /// Asset details (includes is_spatial for Dolby Atmos)
    #[serde(default, deserialize_with = "null_to_default")]
    pub asset_details: Vec<AssetDetail>,

    // === CONTRIBUTORS ===
    /// Authors
    #[serde(default, deserialize_with = "null_to_default")]
    pub authors: Vec<Person>,

    /// Narrators
    #[serde(default, deserialize_with = "null_to_default")]
    pub narrators: Vec<Person>,

    // === RATING ===
    /// Product rating (aggregate)
    #[serde(default)]
    pub rating: Option<RatingInfo>,

    /// User's personal rating (overall)
    #[serde(rename = "customer_review_overall_rating", default)]
    pub my_user_rating_overall: Option<i32>,

    /// User's personal rating (performance)
    #[serde(rename = "customer_review_performance_rating", default)]
    pub my_user_rating_performance: Option<i32>,

    /// User's personal rating (story)
    #[serde(rename = "customer_review_story_rating", default)]
    pub my_user_rating_story: Option<i32>,

    // === SERIES ===
    /// Series information (if book is part of series)
    #[serde(default)]
    pub series: Option<Vec<SeriesInfo>>,

    // === CATEGORIES ===
    /// Category ladders (hierarchical category paths)
    #[serde(
        rename = "category_ladders",
        default,
        deserialize_with = "null_to_default"
    )]
    pub category_ladders: Vec<CategoryLadder>,

    // === IMAGES ===
    /// Product images at various sizes
    #[serde(
        rename = "product_images",
        default,
        deserialize_with = "null_to_default"
    )]
    pub product_images: HashMap<String, String>,

    // === SUPPLEMENTS ===
    /// PDF companion URL
    #[serde(rename = "pdf_url", default)]
    pub pdf_url: Option<String>,

    // === USER STATE ===
    /// Has user finished listening?
    #[serde(rename = "is_finished", default)]
    pub is_finished: Option<bool>,

    // === AVAILABILITY ===
    /// Is downloadable
    #[serde(rename = "is_downloadable", default)]
    pub is_downloadable: Option<bool>,

    /// Is Audible Plus Catalog title
    #[serde(rename = "is_ayce", default)]
    pub is_ayce: Option<bool>,

    /// Subscription plans (API may return null)
    #[serde(default)]
    pub plans: Option<Vec<Plan>>,

    // === RELATIONSHIPS (for episodes/series) ===
    /// Relationships to other products (parent/child)
    #[serde(default)]
    pub relationships: Option<Vec<Relationship>>,

    /// Episode number (for podcast episodes)
    #[serde(rename = "episode_number", default)]
    pub episode_number: Option<i32>,

    // === ORIGIN ===
    /// Original ASIN (for regional variants)
    #[serde(rename = "origin_asin", default)]
    pub origin_asin: Option<String>,
}

impl LibraryItem {
    /// Get full title with subtitle
    /// Reference: BookImporter.cs:106 (TitleWithSubtitle property)
    pub fn title_with_subtitle(&self) -> String {
        match &self.subtitle {
            Some(sub) if !sub.is_empty() => format!("{}: {}", self.title, sub),
            _ => self.title.clone(),
        }
    }

    /// Get content type as enum
    /// Reference: BookImporter.cs:204-212
    pub fn get_content_type(&self) -> ContentType {
        match self.content_type.as_deref() {
            Some("Episode") => ContentType::Episode,
            Some("Parent") => ContentType::Parent,
            Some("Product") | _ => ContentType::Product,
        }
    }

    /// Check if this is an episode
    pub fn is_episode(&self) -> bool {
        matches!(self.get_content_type(), ContentType::Episode)
    }

    /// Check if this is a series parent
    pub fn is_series_parent(&self) -> bool {
        matches!(self.get_content_type(), ContentType::Parent)
    }

    /// Podcast and periodical parents do not carry downloadable audio.
    pub fn is_podcast_parent(&self) -> bool {
        if self.is_series_parent() {
            return true;
        }

        matches!(
            self.content_delivery_type.as_deref(),
            Some("Periodical") | Some("PodcastParent") | Some("PodcastSeries")
        )
    }

    /// Get picture ID (highest quality image)
    /// Reference: BookImporter.cs:156-160
    pub fn get_picture_id(&self) -> Option<String> {
        // Try to get largest image (1215, then 500)
        self.product_images
            .get("1215")
            .or_else(|| self.product_images.get("500"))
            .cloned()
    }

    /// Get large picture URL
    pub fn get_picture_large(&self) -> Option<String> {
        self.product_images.get("500").cloned()
    }

    /// Check if spatial audio (Dolby Atmos)
    /// Reference: BookImporter.cs:169
    pub fn is_spatial(&self) -> bool {
        self.asset_details
            .iter()
            .any(|a| a.is_spatial.unwrap_or(false))
    }

    /// Get publication date (tries multiple date fields)
    pub fn get_publication_date(&self) -> Option<NaiveDate> {
        self.release_date
            .or_else(|| self.publication_datetime.map(|dt| dt.date_naive()))
    }
}

/// Codec information
#[derive(Debug, Clone, Deserialize)]
pub struct CodecInfo {
    /// Codec name (e.g., "aax", "mp4_22_64")
    #[serde(default)]
    pub name: Option<String>,

    /// Enhanced codec format (e.g., "format4")
    #[serde(default)]
    pub enhanced_codec: Option<String>,

    /// Format type (e.g., "Format4")
    #[serde(default)]
    pub format: Option<String>,

    /// Is Kindle enhanced
    #[serde(default)]
    pub is_kindle_enhanced: Option<bool>,
}

/// Asset detail information
#[derive(Debug, Clone, Deserialize)]
pub struct AssetDetail {
    /// Is spatial audio (Dolby Atmos)
    #[serde(rename = "is_spatial", default)]
    pub is_spatial: Option<bool>,

    /// Codec
    #[serde(default)]
    pub codec: Option<String>,

    /// Format
    #[serde(default)]
    pub format: Option<String>,
}

/// Person information (author, narrator)
/// Maps to C# `Person` class in AudibleApi/Common/Person.cs
#[derive(Debug, Clone, Deserialize)]
pub struct Person {
    /// Person's name. Defaulted rather than required: Audible occasionally returns a
    /// contributor with no name, and that must not fail the item.
    #[serde(default)]
    pub name: String,

    /// Audible contributor ID (ASIN)
    #[serde(default)]
    pub asin: Option<String>,
}

/// Rating information
/// Maps to C# `Rating` class in AudibleApi/Common/Rating.cs
#[derive(Debug, Clone, Deserialize)]
pub struct RatingInfo {
    /// Overall rating distribution
    #[serde(rename = "overall_distribution", default)]
    pub overall_distribution: Option<RatingDistribution>,

    /// Performance rating distribution
    #[serde(rename = "performance_distribution", default)]
    pub performance_distribution: Option<RatingDistribution>,

    /// Story rating distribution
    #[serde(rename = "story_distribution", default)]
    pub story_distribution: Option<RatingDistribution>,
}

/// Rating distribution
#[derive(Debug, Clone, Deserialize)]
pub struct RatingDistribution {
    /// Average rating (0.0-5.0)
    #[serde(rename = "average_rating", default)]
    pub average_rating: Option<f32>,

    /// Number of reviews
    #[serde(rename = "num_ratings", default)]
    pub num_ratings: Option<i32>,
}

/// Series information
/// Maps to C# `SeriesInfo` class in AudibleApi/Common/SeriesInfo.cs
#[derive(Debug, Clone, Deserialize)]
pub struct SeriesInfo {
    /// Series ASIN
    /// Defaulted; a series entry without an ASIN is skipped when linking rather than
    /// failing the whole item. Libation had to relax the same check for a real user.
    #[serde(rename = "asin", default)]
    pub series_id: String,

    /// Series title
    #[serde(rename = "title", default)]
    pub title: Option<String>,

    /// Book's position in series (e.g., "1", "2.5", "Book 3")
    #[serde(rename = "sequence", default)]
    pub sequence: Option<String>,
}

/// Category ladder (hierarchical category path)
/// Maps to C# `CategoryLadder` class in AudibleApi/Common/CategoryLadder.cs
#[derive(Debug, Clone, Deserialize)]
pub struct CategoryLadder {
    /// Ladder structure (array of category nodes)
    #[serde(default, deserialize_with = "null_to_default")]
    pub ladder: Vec<CategoryNode>,
}

/// Category node in ladder
#[derive(Debug, Clone, Deserialize)]
pub struct CategoryNode {
    /// Category ID
    #[serde(rename = "id", default)]
    pub category_id: Option<String>,

    /// Category name
    #[serde(default)]
    pub name: Option<String>,
}

/// Subscription plan
#[derive(Debug, Clone, Deserialize)]
pub struct Plan {
    /// Plan type (e.g., "Plus")
    #[serde(rename = "plan_type", default)]
    pub plan_type: Option<String>,

    /// Is AYCE (All You Can Eat) / Plus Catalog
    #[serde(rename = "is_ayce", default)]
    pub is_ayce: Option<bool>,
}

/// Relationship to other products (for episodes/series)
/// Maps to C# `Relationship` class in AudibleApi/Common/Relationship.cs
#[derive(Debug, Clone, Deserialize)]
pub struct Relationship {
    /// Related product ASIN. Defaulted: a malformed relationship must not cost the
    /// book it belongs to.
    #[serde(default)]
    pub asin: String,

    /// Relationship type ("Episode", "Season", etc.)
    #[serde(rename = "relationship_type", default)]
    pub relationship_type: Option<String>,

    /// Relationship to this product ("Parent", "Child")
    #[serde(rename = "relationship_to_product", default)]
    pub relationship_to_product: Option<String>,

    /// Content delivery type
    #[serde(rename = "content_delivery_type", default)]
    pub content_delivery_type: Option<String>,

    /// Sequence number in series/collection (as string, e.g., "1", "2")
    #[serde(default)]
    pub sequence: Option<String>,

    /// SKU identifier
    #[serde(default)]
    pub sku: Option<String>,

    /// SKU lite identifier
    #[serde(rename = "sku_lite", default)]
    pub sku_lite: Option<String>,

    /// Sort order (as string, e.g., "1", "2")
    #[serde(default)]
    pub sort: Option<String>,

    /// Title of related item
    #[serde(default)]
    pub title: Option<String>,

    /// URL to related item
    #[serde(default)]
    pub url: Option<String>,
}

// ============================================================================
// SYNC STATISTICS
// ============================================================================

/// Library sync statistics
/// Reference: ApplicationServices/LibraryCommands.cs:104-149
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SyncStats {
    /// Total items fetched from API in this sync
    pub total_items: i32,

    /// Total books in your Audible library (from API total_results)
    pub total_library_count: i32,

    /// New books added to database
    pub books_added: i32,

    /// Existing books updated
    pub books_updated: i32,

    /// Books marked as absent (removed from library)
    pub books_absent: i32,

    /// Errors encountered during sync (non-fatal)
    pub errors: Vec<String>,

    /// Items the API returned that could not be parsed or imported. These are the
    /// silent losses: the page succeeded, the book did not arrive.
    pub items_failed: i32,

    /// Whether there are more pages to fetch (for pagination)
    pub has_more: bool,
}

impl SyncStats {
    pub fn new() -> Self {
        Self::default()
    }
}

// ============================================================================
// LIBRARY SYNC IMPLEMENTATION
// ============================================================================

impl AudibleClient {
    /// Synchronize library from Audible API
    ///
    /// This is the main entry point for library sync. It fetches all pages from the
    /// Audible library API, converts items to database models, and upserts to the database.
    ///
    /// # Reference
    /// Based on `ImportAccountAsync()` - ApplicationServices/LibraryCommands.cs:104-181
    ///
    /// # Process
    /// 1. Fetch all library items from Audible API (paginated)
    /// 2. Convert API items to Book models
    /// 3. Upsert books into database
    /// 4. Create LibraryBook records (account ownership)
    /// 5. Link contributors (authors, narrators, publishers)
    /// 6. Link series with order
    /// 7. Link categories
    /// 8. Mark absent books (removed from library since last scan)
    ///
    /// # Arguments
    /// * `db` - Database connection
    /// * `account` - Account to sync for
    ///
    /// # Returns
    /// Sync statistics (total count, new count, errors)
    ///
    /// # Errors
    /// Returns error if:
    /// - API request fails
    /// - Database operations fail
    /// - Validation errors prevent import
    pub async fn sync_library(&mut self, db: &Database, account: &Account) -> Result<SyncStats> {
        let mut stats = SyncStats::new();

        // Fetch all library items from API
        let options = LibraryOptions::default();
        let (items, total_count) = self.fetch_all_library_items(options).await?;

        stats.total_items = items.len() as i32;
        stats.total_library_count = total_count;

        if items.is_empty() {
            return Ok(stats);
        }

        // Import items into database
        let (new_count, updated_count, errors) = self
            .import_items_to_db(db, &items, &account.account_id)
            .await?;

        stats.books_added = new_count;
        stats.books_updated = updated_count;
        stats.errors = errors;

        // Mark absent books (removed from library)
        let absent_count = self
            .mark_absent_books(db, &items, &account.account_id)
            .await?;
        stats.books_absent = absent_count;

        Ok(stats)
    }

    /// Synchronize a single page of library from Audible API
    ///
    /// This allows for progressive UI updates by syncing page-by-page instead of all at once.
    /// The UI can display progress and update the book list incrementally.
    ///
    /// # Arguments
    /// * `db` - Database connection
    /// * `account` - Account with authentication credentials
    /// * `page` - Page number to fetch (1-indexed)
    ///
    /// # Returns
    /// * `SyncStats` - Statistics for this page, including `has_more` flag
    ///
    /// # Example
    /// ```rust,ignore
    /// let mut page = 1;
    /// loop {
    ///     let stats = client.sync_library_page(&db, &account, page).await?;
    ///     println!("Page {}: {} items", page, stats.total_items);
    ///     if !stats.has_more {
    ///         break;
    ///     }
    ///     page += 1;
    /// }
    /// ```
    pub async fn sync_library_page(
        &mut self,
        db: &Database,
        account: &Account,
        page: i32,
    ) -> Result<SyncStats> {
        let mut stats = SyncStats::new();

        // Fetch single page from API
        let mut options = LibraryOptions::default();
        options.page_number = page;

        // Parse the envelope loosely: one malformed item must cost one book, not the
        // whole page (and, because an empty/failed page ends pagination, not the whole
        // rest of the library).
        let mut raw: serde_json::Value = self.get_with_query("/1.0/library", &options).await?;
        let mut raw_items = Self::take_raw_items(&mut raw);

        // An empty page is how this sync knows it has reached the end — Audible's library
        // response carries no total to check against. A transient empty-but-successful
        // response would therefore truncate the library and report success, so confirm it
        // with a second request before believing it.
        if raw_items.is_empty() {
            let mut confirmation: serde_json::Value =
                self.get_with_query("/1.0/library", &options).await?;
            raw_items = Self::take_raw_items(&mut confirmation);
            if !raw_items.is_empty() {
                stats.errors.push(format!(
                    "Page {page} returned no items on the first attempt but {} on retry \
                     — the empty response was transient",
                    raw_items.len()
                ));
            }
        }

        stats.total_items = raw_items.len() as i32;

        // Audible does not return `total_results` for /1.0/library (verified against
        // captured responses), so the only end-of-library signal is an empty page.
        if let Some(total) = raw
            .get("total_results")
            .and_then(serde_json::Value::as_i64)
        {
            stats.total_library_count = total as i32;
        }
        stats.has_more = !raw_items.is_empty();

        if raw_items.is_empty() {
            return Ok(stats);
        }

        // Convert item-by-item; a failure names its ASIN and is counted, never silent.
        let mut items = Vec::with_capacity(raw_items.len());
        for raw_item in raw_items {
            let asin = raw_item
                .get("asin")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("<unknown asin>")
                .to_string();
            match serde_json::from_value::<LibraryItem>(raw_item) {
                Ok(item) => items.push(item),
                Err(e) => {
                    stats.items_failed += 1;
                    stats
                        .errors
                        .push(format!("Page {page}: could not read item '{asin}': {e}"));
                }
            }
        }

        if items.is_empty() {
            // Every item on a non-empty page failed to parse. has_more stays true so the
            // sync keeps going rather than treating this as the end of the library.
            return Ok(stats);
        }

        // Import items into database
        let (new_count, updated_count, errors) = self
            .import_items_to_db(db, &items, &account.account_id)
            .await?;

        stats.books_added = new_count;
        stats.books_updated = updated_count;
        stats.items_failed += errors.len() as i32;
        stats.errors.extend(errors);

        // Note: books_absent is only calculated at the end of full sync
        // Individual pages don't mark absent books

        Ok(stats)
    }

    /// Take the `items` array out of a raw library response, leaving the rest of the
    /// envelope intact. Missing or non-array `items` reads as an empty page.
    fn take_raw_items(raw: &mut serde_json::Value) -> Vec<serde_json::Value> {
        match raw.get_mut("items").map(serde_json::Value::take) {
            Some(serde_json::Value::Array(items)) => items,
            _ => Vec::new(),
        }
    }

    /// Fetch and import one page of episodes for a podcast or periodical parent.
    pub async fn sync_podcast_episodes_page(
        &self,
        db: &Database,
        account: &Account,
        parent_asin: &str,
        offset: i32,
        limit: i32,
    ) -> Result<SyncStats> {
        let mut stats = SyncStats::new();
        let child_asins = self.fetch_podcast_episode_asins(parent_asin).await?;

        stats.total_library_count = child_asins.len() as i32;

        // On the first page, remove previously-imported episodes that Audible no
        // longer lists as children (e.g. season grouping nodes that older versions
        // ingested as fake 0-length episodes). Guarded by a non-empty result so a
        // transient empty/failed response can't wipe a podcast's episodes.
        if offset <= 0 && !child_asins.is_empty() {
            self.prune_removed_episodes(db, parent_asin, &child_asins)
                .await?;
        }

        if child_asins.is_empty() {
            return Ok(stats);
        }

        let page_offset = offset.max(0) as usize;
        if page_offset >= child_asins.len() {
            return Ok(stats);
        }

        let page_limit = limit.clamp(1, 100) as usize;
        let page_end = (page_offset + page_limit).min(child_asins.len());
        let page_asins = child_asins[page_offset..page_end].to_vec();

        stats.total_items = page_asins.len() as i32;
        stats.has_more = page_end < child_asins.len();

        let new_count = self
            .import_episodes(db, parent_asin, &page_asins, &account.account_id)
            .await?;

        stats.books_added = new_count;
        stats.books_updated = stats.total_items.saturating_sub(new_count);

        Ok(stats)
    }

    /// Delete episodes previously imported for `parent_asin` that are no longer in
    /// the current child list. Cleans up stale rows such as podcast season nodes.
    async fn prune_removed_episodes(
        &self,
        db: &Database,
        parent_asin: &str,
        child_asins: &[String],
    ) -> Result<()> {
        let pool = db.pool();

        let existing: Vec<(i64, String)> = sqlx::query_as(
            "SELECT book_id, audible_product_id FROM Books \
             WHERE origin_asin = ? AND content_type = ? AND audible_product_id != ?",
        )
        .bind(parent_asin)
        .bind(ContentType::Episode as i32)
        .bind(parent_asin)
        .fetch_all(pool)
        .await?;

        let keep: HashSet<&str> = child_asins.iter().map(|asin| asin.as_str()).collect();

        for (book_id, asin) in existing {
            if !keep.contains(asin.as_str()) {
                crate::storage::queries::delete_book(pool, book_id).await?;
            }
        }

        Ok(())
    }

    async fn fetch_podcast_episode_asins(&self, parent_asin: &str) -> Result<Vec<String>> {
        let response_groups = "relationships,product_desc,media,product_extended_attrs";
        let endpoint = format!(
            "/1.0/catalog/products/{}?response_groups={}&image_sizes=500",
            parent_asin,
            urlencoding::encode(response_groups)
        );
        let response: serde_json::Value = self.get(&endpoint).await?;
        let relationships = response
            .get("product")
            .and_then(|product| product.get("relationships"))
            .and_then(|relationships| relationships.as_array());

        let Some(relationships) = relationships else {
            return Ok(Vec::new());
        };

        let mut seen = HashSet::new();
        let mut child_asins: Vec<(String, i32)> = Vec::new();

        for relationship in relationships {
            let relationship_to_product = relationship
                .get("relationship_to_product")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let relationship_type = relationship
                .get("relationship_type")
                .and_then(|value| value.as_str())
                .unwrap_or("");

            // Skip season grouping nodes: Audible returns them as children of the
            // podcast, but they have no audio (0 length) and produce errors if
            // treated as downloadable episodes.
            if relationship_type.eq_ignore_ascii_case("season") {
                continue;
            }

            let is_child = relationship_to_product.eq_ignore_ascii_case("child")
                || relationship_type.eq_ignore_ascii_case("episode");

            if !is_child {
                continue;
            }

            let Some(asin) = relationship.get("asin").and_then(|value| value.as_str()) else {
                continue;
            };

            if asin.is_empty() || asin == parent_asin || !seen.insert(asin.to_string()) {
                continue;
            }

            let sort = relationship
                .get("sort")
                .and_then(|value| {
                    value
                        .as_i64()
                        .or_else(|| value.as_str().and_then(|text| text.parse::<i64>().ok()))
                })
                .unwrap_or(0) as i32;
            child_asins.push((asin.to_string(), sort));
        }

        child_asins.sort_by(|left, right| right.1.cmp(&left.1));

        Ok(child_asins.into_iter().map(|(asin, _sort)| asin).collect())
    }

    /// Fetch all library items from Audible API with pagination
    ///
    /// # Reference
    /// Based on `scanAccountsAsync()` and `getItemsAsync()` - ApiExtended.cs:84-165
    ///
    /// # Process
    /// 1. Fetch first page to get total count
    /// 2. Calculate number of pages needed
    /// 3. Fetch remaining pages concurrently (respecting rate limits)
    /// 4. Merge all items into single collection
    ///
    /// # Arguments
    /// * `options` - Library query options (page size, filters, response groups)
    ///
    /// # Returns
    /// All library items across all pages
    ///
    /// # Errors
    /// Returns error if API requests fail
    async fn fetch_all_library_items(
        &mut self,
        mut options: LibraryOptions,
    ) -> Result<(Vec<LibraryItem>, i32)> {
        let mut all_items = Vec::new();

        // Fetch first page
        options.page_number = 1;
        let first_response: LibraryResponse = self.get_with_query("/1.0/library", &options).await?;

        all_items.extend(first_response.items);

        // If API provides total_results, use it for pagination
        if let Some(total) = first_response.total_results {
            let page_size = options.number_of_results_per_page;
            let total_pages = (total as f32 / page_size as f32).ceil() as i32;

            // Fetch remaining pages
            for page_num in 2..=total_pages {
                options.page_number = page_num;
                let response: LibraryResponse =
                    self.get_with_query("/1.0/library", &options).await?;

                all_items.extend(response.items);
            }

            Ok((all_items, total))
        } else {
            // API doesn't provide total - keep fetching until empty response
            let page_size = options.number_of_results_per_page;
            let mut page_num = 2;

            loop {
                options.page_number = page_num;
                let response: LibraryResponse =
                    self.get_with_query("/1.0/library", &options).await?;

                if response.items.is_empty() {
                    break;
                }

                all_items.extend(response.items);
                page_num += 1;

                // Safety limit to prevent infinite loop
                if page_num > 1000 {
                    break;
                }
            }

            let total = all_items.len() as i32;
            Ok((all_items, total))
        }
    }

    /// Import library items into database
    ///
    /// # Reference
    /// Based on `importIntoDbAsync()` - ApplicationServices/LibraryCommands.cs:350-366
    /// And `LibraryBookImporter.DoImport()` - DtoImporterService/LibraryBookImporter.cs:22-28
    ///
    /// # Arguments
    /// * `db` - Database connection
    /// * `items` - Library items from API
    /// * `account_id` - Account ID for LibraryBook records
    ///
    /// # Returns
    /// Tuple of (new_count, updated_count, errors)
    async fn import_items_to_db(
        &self,
        db: &Database,
        items: &[LibraryItem],
        account_id: &str,
    ) -> Result<(i32, i32, Vec<String>)> {
        let mut new_count = 0;
        let mut updated_count = 0;
        let mut errors = Vec::new();

        // Build lookup maps for contributors and series
        let mut contributor_cache: HashMap<String, i64> = HashMap::new();
        let mut series_cache: HashMap<String, i64> = HashMap::new();

        // Import contributors first (authors, narrators, publishers)
        for item in items {
            for author in &item.authors {
                if !contributor_cache.contains_key(&author.name) {
                    match self
                        .upsert_contributor(db, &author.name, author.asin.as_deref())
                        .await
                    {
                        Ok(id) => {
                            contributor_cache.insert(author.name.clone(), id);
                        }
                        Err(e) => {
                            errors.push(format!("Failed to import author '{}': {}", author.name, e))
                        }
                    }
                }
            }

            for narrator in &item.narrators {
                if !contributor_cache.contains_key(&narrator.name) {
                    match self
                        .upsert_contributor(db, &narrator.name, narrator.asin.as_deref())
                        .await
                    {
                        Ok(id) => {
                            contributor_cache.insert(narrator.name.clone(), id);
                        }
                        Err(e) => errors.push(format!(
                            "Failed to import narrator '{}': {}",
                            narrator.name, e
                        )),
                    }
                }
            }

            if let Some(ref publisher) = item.publisher {
                if !contributor_cache.contains_key(publisher) {
                    match self.upsert_contributor(db, publisher, None).await {
                        Ok(id) => {
                            contributor_cache.insert(publisher.clone(), id);
                        }
                        Err(e) => errors
                            .push(format!("Failed to import publisher '{}': {}", publisher, e)),
                    }
                }
            }
        }

        // Import series
        for item in items {
            if let Some(series_list) = &item.series {
                for series_info in series_list {
                    if !series_cache.contains_key(&series_info.series_id) {
                        match self
                            .upsert_series(db, &series_info.series_id, series_info.title.as_deref())
                            .await
                        {
                            Ok(id) => {
                                series_cache.insert(series_info.series_id.clone(), id);
                            }
                            Err(e) => errors.push(format!(
                                "Failed to import series '{}': {}",
                                series_info.series_id, e
                            )),
                        }
                    }
                }
            }
        }

        // Import books and link relationships
        for item in items {
            match self
                .import_book(db, item, account_id, &contributor_cache, &series_cache)
                .await
            {
                Ok(is_new) => {
                    if is_new {
                        new_count += 1;
                    } else {
                        updated_count += 1;
                    }
                }
                Err(e) => {
                    errors.push(format!("Failed to import book '{}': {}", item.asin, e));
                }
            }
        }

        for item in items {
            if !item.is_podcast_parent() {
                continue;
            }

            let child_asins: Vec<String> = item
                .relationships
                .as_ref()
                .map(|relationships| {
                    relationships
                        .iter()
                        .filter(|relationship| {
                            relationship
                                .relationship_to_product
                                .as_deref()
                                .map(|value| value.eq_ignore_ascii_case("child"))
                                .unwrap_or(false)
                        })
                        .map(|relationship| relationship.asin.clone())
                        .collect()
                })
                .unwrap_or_default();

            if child_asins.is_empty() {
                continue;
            }

            match self
                .import_episodes(db, &item.asin, &child_asins, account_id)
                .await
            {
                Ok(count) => new_count += count,
                Err(e) => errors.push(format!(
                    "Failed to import episodes for '{}': {}",
                    item.asin, e
                )),
            }
        }

        Ok((new_count, updated_count, errors))
    }

    async fn import_episodes(
        &self,
        db: &Database,
        parent_asin: &str,
        child_asins: &[String],
        account_id: &str,
    ) -> Result<i32> {
        let mut new_count = 0;

        for chunk in child_asins.chunks(crate::api::client::BATCH_SIZE) {
            let mut products = self.get_catalog_products_batch(chunk.to_vec()).await?;
            let mut loaded_asins: HashSet<String> = products
                .iter()
                .map(|product| product.asin.clone())
                .collect();

            let missing_asins: Vec<String> = chunk
                .iter()
                .filter(|asin| !loaded_asins.contains(*asin))
                .cloned()
                .collect();

            if !missing_asins.is_empty() {
                use futures_util::stream::{self, StreamExt};

                let fallback_products = stream::iter(missing_asins)
                    .map(|asin| async move {
                        let result = self.get_catalog_product(&asin).await;
                        (asin, result)
                    })
                    .buffer_unordered(8)
                    .collect::<Vec<_>>()
                    .await;

                for (asin, result) in fallback_products {
                    match result {
                        Ok(product) => {
                            loaded_asins.insert(product.asin.clone());
                            products.push(product);
                        }
                        Err(error) => {
                            eprintln!(
                                "Warning: Failed to fetch podcast episode '{}': {}",
                                asin, error
                            );
                        }
                    }
                }
            }

            for product in &products {
                if self
                    .import_episode(db, product, parent_asin, account_id)
                    .await?
                {
                    new_count += 1;
                }
            }
        }

        Ok(new_count)
    }

    async fn import_episode(
        &self,
        db: &Database,
        product: &crate::api::content::CatalogProduct,
        parent_asin: &str,
        account_id: &str,
    ) -> Result<bool> {
        let pool = db.pool();

        let existing: Option<(i64,)> =
            sqlx::query_as("SELECT book_id FROM Books WHERE audible_product_id = ?")
                .bind(&product.asin)
                .fetch_optional(pool)
                .await?;
        let is_new = existing.is_none();

        let description = product.publisher_summary.as_deref().unwrap_or("");
        let is_abridged = product.format_type.eq_ignore_ascii_case("abridged");
        let date_published = product
            .release_date
            .or_else(|| product.publication_datetime.map(|dt| dt.date_naive()));
        let picture = product
            .product_images
            .as_ref()
            .and_then(|images| images.get("500").cloned());
        let rating_overall = product
            .rating
            .as_ref()
            .and_then(|rating| rating.overall_distribution.as_ref())
            .map(|distribution| distribution.average_rating)
            .unwrap_or(0.0);
        let locale = if product.language.is_empty() {
            "en_US"
        } else {
            product.language.as_str()
        };

        let book_id = match existing {
            Some((id,)) => {
                sqlx::query(
                    r#"
                    UPDATE Books
                    SET title = ?, subtitle = ?, description = ?, length_in_minutes = ?,
                        content_type = ?, picture_id = ?, picture_large = ?, is_abridged = ?,
                        date_published = ?, language = ?, rating_overall = ?,
                        is_downloadable = 1, origin_asin = ?, episode_number = ?,
                        content_delivery_type = 'PodcastEpisode', updated_at = datetime('now')
                    WHERE book_id = ?
                    "#,
                )
                .bind(&product.title)
                .bind(&product.subtitle)
                .bind(description)
                .bind(product.runtime_length_min)
                .bind(ContentType::Episode as i32)
                .bind(&picture)
                .bind(&picture)
                .bind(is_abridged)
                .bind(date_published)
                .bind(locale)
                .bind(rating_overall)
                .bind(parent_asin)
                .bind(product.episode_number)
                .bind(id)
                .execute(pool)
                .await?;
                id
            }
            None => {
                let result = sqlx::query(
                    r#"
                    INSERT INTO Books (
                        audible_product_id, title, subtitle, description, length_in_minutes,
                        content_type, locale, picture_id, picture_large, is_abridged, is_spatial,
                        date_published, language, rating_overall, rating_performance, rating_story,
                        pdf_url, is_finished, is_downloadable, is_ayce, origin_asin, episode_number,
                        content_delivery_type, created_at, updated_at
                    )
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?, ?, 0, 0, NULL, 0, 1, 0, ?, ?, 'PodcastEpisode', datetime('now'), datetime('now'))
                    "#,
                )
                .bind(&product.asin)
                .bind(&product.title)
                .bind(&product.subtitle)
                .bind(description)
                .bind(product.runtime_length_min)
                .bind(ContentType::Episode as i32)
                .bind(locale)
                .bind(&picture)
                .bind(&picture)
                .bind(is_abridged)
                .bind(date_published)
                .bind(locale)
                .bind(rating_overall)
                .bind(parent_asin)
                .bind(product.episode_number)
                .execute(pool)
                .await?;
                result.last_insert_rowid()
            }
        };

        let date_added = product
            .publication_datetime
            .unwrap_or_else(chrono::Utc::now);
        self.upsert_library_book(db, book_id, account_id, &date_added)
            .await?;

        let user_item_exists: Option<(i64,)> =
            sqlx::query_as("SELECT book_id FROM UserDefinedItems WHERE book_id = ?")
                .bind(book_id)
                .fetch_optional(pool)
                .await?;

        if user_item_exists.is_none() {
            sqlx::query(
                r#"
                INSERT INTO UserDefinedItems (
                    book_id, tags, user_rating_overall, user_rating_performance, user_rating_story,
                    book_status, pdf_status, is_finished
                )
                VALUES (?, '', 0, 0, 0, 0, NULL, 0)
                "#,
            )
            .bind(book_id)
            .execute(pool)
            .await?;
        }

        sqlx::query("DELETE FROM BookContributors WHERE book_id = ?")
            .bind(book_id)
            .execute(pool)
            .await?;

        for (order, author) in product.authors.iter().enumerate() {
            let contributor_id = self
                .upsert_contributor(db, &author.name, Some(author.asin.as_str()))
                .await?;
            sqlx::query(
                r#"INSERT OR IGNORE INTO BookContributors (book_id, contributor_id, role, "order") VALUES (?, ?, ?, ?)"#,
            )
            .bind(book_id)
            .bind(contributor_id)
            .bind(Role::Author as i32)
            .bind(order as i16)
            .execute(pool)
            .await?;
        }

        let narrators = if product.narrators.is_empty() {
            &product.authors
        } else {
            &product.narrators
        };

        for (order, narrator) in narrators.iter().enumerate() {
            let contributor_id = self
                .upsert_contributor(db, &narrator.name, Some(narrator.asin.as_str()))
                .await?;
            sqlx::query(
                r#"INSERT OR IGNORE INTO BookContributors (book_id, contributor_id, role, "order") VALUES (?, ?, ?, ?)"#,
            )
            .bind(book_id)
            .bind(contributor_id)
            .bind(Role::Narrator as i32)
            .bind(order as i16)
            .execute(pool)
            .await?;
        }

        Ok(is_new)
    }

    /// Import a single book into database
    ///
    /// # Reference
    /// Based on `BookImporter.DoImport()` - DtoImporterService/BookImporter.cs:28-72
    ///
    /// # Arguments
    /// * `db` - Database connection
    /// * `item` - Library item from API
    /// * `account_id` - Account ID
    /// * `contributor_cache` - Contributor name -> ID mapping
    /// * `series_cache` - Series ASIN -> ID mapping
    ///
    /// # Returns
    /// `true` if book was newly created, `false` if updated
    async fn import_book(
        &self,
        db: &Database,
        item: &LibraryItem,
        account_id: &str,
        contributor_cache: &HashMap<String, i64>,
        series_cache: &HashMap<String, i64>,
    ) -> Result<bool> {
        let pool = db.pool();

        // Check if book exists
        let existing: Option<(i64,)> =
            sqlx::query_as("SELECT book_id FROM Books WHERE audible_product_id = ?")
                .bind(&item.asin)
                .fetch_optional(pool)
                .await?;

        let (book_id, is_new) = match existing {
            Some((id,)) => {
                // Update existing book
                self.update_book(db, id, item).await?;
                (id, false)
            }
            None => {
                // Create new book
                let id = self.create_book(db, item).await?;
                (id, true)
            }
        };

        // Upsert LibraryBook record
        // No purchase date (or an unusable one) still deserves a library row; use the
        // moment we first saw the book so ordering by "date added" stays sane.
        let date_added = item.purchase_date.unwrap_or_else(Utc::now);
        self.upsert_library_book(db, book_id, account_id, &date_added)
            .await?;

        // Link contributors (authors, narrators, publisher)
        self.link_contributors(db, book_id, item, contributor_cache)
            .await?;

        // Link series
        self.link_series(db, book_id, item, series_cache).await?;

        // Link categories/genres
        self.link_categories(db, book_id, item).await?;

        // Update user-defined metadata
        self.update_user_defined_item(db, book_id, item).await?;

        Ok(is_new)
    }

    /// Link category ladders to a book for the genre filter.
    ///
    /// Each ladder is stored once in CategoryLadders (keyed by its category
    /// ids) with `ladder` as a JSON array of category names, then linked via
    /// BookCategories.
    async fn link_categories(&self, db: &Database, book_id: i64, item: &LibraryItem) -> Result<()> {
        let pool = db.pool();

        sqlx::query("DELETE FROM BookCategories WHERE book_id = ?")
            .bind(book_id)
            .execute(pool)
            .await?;

        for ladder in &item.category_ladders {
            let names: Vec<&str> = ladder
                .ladder
                .iter()
                .filter_map(|node| node.name.as_deref())
                .filter(|name| !name.trim().is_empty())
                .collect();
            if names.is_empty() {
                continue;
            }

            let ids: Vec<&str> = ladder
                .ladder
                .iter()
                .filter_map(|node| node.category_id.as_deref())
                .collect();
            let ladder_key = if ids.is_empty() {
                names.join("|")
            } else {
                ids.join("|")
            };
            let ladder_json = serde_json::to_string(&names).unwrap_or_else(|_| "[]".to_string());

            let ladder_id = crate::storage::queries::upsert_category_ladder(
                pool,
                &NewCategoryLadder {
                    audible_ladder_id: ladder_key,
                    ladder: ladder_json,
                },
            )
            .await?;

            crate::storage::queries::add_book_category(pool, book_id, ladder_id).await?;
        }

        Ok(())
    }

    /// Create new book record
    ///
    /// # Reference
    /// Based on `BookImporter.createNewBook()` - DtoImporterService/BookImporter.cs:74-144
    async fn create_book(&self, db: &Database, item: &LibraryItem) -> Result<i64> {
        let pool = db.pool();

        let content_type = item.get_content_type() as i32;
        // Prefer the full publisher summary; merchandising_summary is a teaser
        let description = item
            .publisher_summary
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .or(item.description.as_deref())
            .unwrap_or("");
        let length_in_minutes = item.length_in_minutes.unwrap_or(0);
        let is_abridged = item.is_abridged.unwrap_or(false);
        let is_spatial = item.is_spatial();
        let language = item.language.as_deref();
        let date_published = item.get_publication_date();

        let rating = item.rating.as_ref();
        let rating_overall = rating
            .and_then(|r| r.overall_distribution.as_ref())
            .and_then(|d| d.average_rating)
            .unwrap_or(0.0);
        let rating_performance = rating
            .and_then(|r| r.performance_distribution.as_ref())
            .and_then(|d| d.average_rating)
            .unwrap_or(0.0);
        let rating_story = rating
            .and_then(|r| r.story_distribution.as_ref())
            .and_then(|d| d.average_rating)
            .unwrap_or(0.0);

        let picture_id = item.get_picture_id();
        let picture_large = item.get_picture_large();

        // Determine locale from language
        let locale = language.unwrap_or("en_US");

        // Extract new fields
        let pdf_url = item.pdf_url.as_deref();
        let is_finished = item.is_finished.unwrap_or(false);
        let is_downloadable = if item.is_podcast_parent() {
            false
        } else {
            item.is_downloadable.unwrap_or(true)
        };
        let is_ayce = item.is_ayce.unwrap_or(false);
        let origin_asin = item.origin_asin.as_deref();
        let episode_number = item.episode_number;
        let content_delivery_type = item.content_delivery_type.as_deref();

        let result = sqlx::query(
            r#"
            INSERT INTO Books (
                audible_product_id, title, subtitle, description, length_in_minutes,
                content_type, locale, picture_id, picture_large, is_abridged, is_spatial,
                date_published, language, rating_overall, rating_performance, rating_story,
                pdf_url, is_finished, is_downloadable, is_ayce, origin_asin, episode_number,
                content_delivery_type, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'))
            "#
        )
        .bind(&item.asin)
        .bind(&item.title)
        .bind(&item.subtitle)
        .bind(description)
        .bind(length_in_minutes)
        .bind(content_type)
        .bind(locale)
        .bind(picture_id)
        .bind(picture_large)
        .bind(is_abridged)
        .bind(is_spatial)
        .bind(date_published)
        .bind(language)
        .bind(rating_overall)
        .bind(rating_performance)
        .bind(rating_story)
        .bind(pdf_url)
        .bind(is_finished)
        .bind(is_downloadable)
        .bind(is_ayce)
        .bind(origin_asin)
        .bind(episode_number)
        .bind(content_delivery_type)
        .execute(pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    /// Update existing book record
    ///
    /// # Reference
    /// Based on `BookImporter.updateBook()` - DtoImporterService/BookImporter.cs:146-202
    async fn update_book(&self, db: &Database, book_id: i64, item: &LibraryItem) -> Result<()> {
        let pool = db.pool();

        let length_in_minutes = item.length_in_minutes.unwrap_or(0);
        let is_abridged = item.is_abridged.unwrap_or(false);
        let is_spatial = item.is_spatial();
        let language = item.language.as_deref();
        let date_published = item.get_publication_date();

        let rating = item.rating.as_ref();
        let rating_overall = rating
            .and_then(|r| r.overall_distribution.as_ref())
            .and_then(|d| d.average_rating)
            .unwrap_or(0.0);
        let rating_performance = rating
            .and_then(|r| r.performance_distribution.as_ref())
            .and_then(|d| d.average_rating)
            .unwrap_or(0.0);
        let rating_story = rating
            .and_then(|r| r.story_distribution.as_ref())
            .and_then(|d| d.average_rating)
            .unwrap_or(0.0);

        let picture_id = item.get_picture_id();
        let picture_large = item.get_picture_large();

        // Extract new fields
        let pdf_url = item.pdf_url.as_deref();
        let is_finished = item.is_finished.unwrap_or(false);
        let is_downloadable = if item.is_podcast_parent() {
            false
        } else {
            item.is_downloadable.unwrap_or(true)
        };
        let is_ayce = item.is_ayce.unwrap_or(false);
        let origin_asin = item.origin_asin.as_deref();
        let episode_number = item.episode_number;
        let content_delivery_type = item.content_delivery_type.as_deref();
        // Prefer the full publisher summary; merchandising_summary is a teaser.
        // COALESCE(NULLIF(...)) below keeps an existing description when the
        // API returns none.
        let description = item
            .publisher_summary
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .or(item.description.as_deref())
            .unwrap_or("");

        sqlx::query(
            r#"
            UPDATE Books
            SET title = ?, subtitle = ?, length_in_minutes = ?, is_abridged = ?, is_spatial = ?,
                date_published = ?, language = ?, picture_id = ?, picture_large = ?,
                rating_overall = ?, rating_performance = ?, rating_story = ?,
                pdf_url = ?, is_finished = ?, is_downloadable = ?, is_ayce = ?,
                origin_asin = ?, episode_number = ?, content_delivery_type = ?,
                description = COALESCE(NULLIF(?, ''), description),
                updated_at = datetime('now')
            WHERE book_id = ?
            "#,
        )
        .bind(&item.title)
        .bind(&item.subtitle)
        .bind(length_in_minutes)
        .bind(is_abridged)
        .bind(is_spatial)
        .bind(date_published)
        .bind(language)
        .bind(picture_id)
        .bind(picture_large)
        .bind(rating_overall)
        .bind(rating_performance)
        .bind(rating_story)
        .bind(pdf_url)
        .bind(is_finished)
        .bind(is_downloadable)
        .bind(is_ayce)
        .bind(origin_asin)
        .bind(episode_number)
        .bind(content_delivery_type)
        .bind(description)
        .bind(book_id)
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Upsert LibraryBook record (account ownership)
    ///
    /// # Reference
    /// Based on `LibraryBookImporter.upsertLibraryBooks()` - DtoImporterService/LibraryBookImporter.cs:30-96
    async fn upsert_library_book(
        &self,
        db: &Database,
        book_id: i64,
        account_id: &str,
        date_added: &DateTime<Utc>,
    ) -> Result<()> {
        let pool = db.pool();

        sqlx::query(
            r#"
            INSERT INTO BookAccounts (book_id, account, date_added, is_deleted, absent_from_last_scan)
            VALUES (?, ?, ?, 0, 0)
            ON CONFLICT(book_id, account) DO UPDATE SET
                date_added = excluded.date_added,
                is_deleted = 0,
                absent_from_last_scan = 0
            "#
        )
        .bind(book_id)
        .bind(account_id)
        .bind(date_added)
        .execute(pool)
        .await?;

        // Check if LibraryBook exists
        let exists: Option<(bool,)> =
            sqlx::query_as("SELECT is_deleted FROM LibraryBooks WHERE book_id = ?")
                .bind(book_id)
                .fetch_optional(pool)
                .await?;

        match exists {
            Some(_) => {
                // Keep the legacy single-account row alive without replacing ownership.
                sqlx::query(
                    r#"
                    UPDATE LibraryBooks
                    SET absent_from_last_scan = 0, is_deleted = 0
                    WHERE book_id = ?
                    "#,
                )
                .bind(book_id)
                .execute(pool)
                .await?;
            }
            None => {
                // Insert new LibraryBook
                sqlx::query(
                    r#"
                    INSERT INTO LibraryBooks (book_id, date_added, account, is_deleted, absent_from_last_scan)
                    VALUES (?, ?, ?, 0, 0)
                    "#
                )
                .bind(book_id)
                .bind(date_added)
                .bind(account_id)
                .execute(pool)
                .await?;
            }
        }

        Ok(())
    }

    /// Link contributors to book (authors, narrators, publisher)
    ///
    /// # Reference
    /// Based on `BookImporter.createNewBook()` - DtoImporterService/BookImporter.cs:85-138
    async fn link_contributors(
        &self,
        db: &Database,
        book_id: i64,
        item: &LibraryItem,
        contributor_cache: &HashMap<String, i64>,
    ) -> Result<()> {
        let pool = db.pool();

        // Delete existing contributor links
        sqlx::query("DELETE FROM BookContributors WHERE book_id = ?")
            .bind(book_id)
            .execute(pool)
            .await?;

        // Link authors
        for (order, author) in item.authors.iter().enumerate() {
            if let Some(&contributor_id) = contributor_cache.get(&author.name) {
                sqlx::query(
                    r#"
                    INSERT INTO BookContributors (book_id, contributor_id, role, "order")
                    VALUES (?, ?, ?, ?)
                    "#,
                )
                .bind(book_id)
                .bind(contributor_id)
                .bind(Role::Author as i32)
                .bind(order as i16)
                .execute(pool)
                .await?;
            }
        }

        // Link narrators
        let narrators = if item.narrators.is_empty() {
            // If no narrators, authors are narrators
            &item.authors
        } else {
            &item.narrators
        };

        for (order, narrator) in narrators.iter().enumerate() {
            if let Some(&contributor_id) = contributor_cache.get(&narrator.name) {
                sqlx::query(
                    r#"
                    INSERT INTO BookContributors (book_id, contributor_id, role, "order")
                    VALUES (?, ?, ?, ?)
                    "#,
                )
                .bind(book_id)
                .bind(contributor_id)
                .bind(Role::Narrator as i32)
                .bind(order as i16)
                .execute(pool)
                .await?;
            }
        }

        // Link publisher
        if let Some(ref publisher_name) = item.publisher {
            if let Some(&contributor_id) = contributor_cache.get(publisher_name) {
                sqlx::query(
                    r#"
                    INSERT INTO BookContributors (book_id, contributor_id, role, "order")
                    VALUES (?, ?, ?, 0)
                    "#,
                )
                .bind(book_id)
                .bind(contributor_id)
                .bind(Role::Publisher as i32)
                .execute(pool)
                .await?;
            }
        }

        Ok(())
    }

    /// Link series to book
    ///
    /// # Reference
    /// Based on `BookImporter.updateBook()` - DtoImporterService/BookImporter.cs:179-188
    async fn link_series(
        &self,
        db: &Database,
        book_id: i64,
        item: &LibraryItem,
        series_cache: &HashMap<String, i64>,
    ) -> Result<()> {
        let pool = db.pool();

        // Delete existing series links
        sqlx::query("DELETE FROM SeriesBooks WHERE book_id = ?")
            .bind(book_id)
            .execute(pool)
            .await?;

        // Link series
        if let Some(series_list) = &item.series {
            for series_info in series_list {
                if let Some(&series_id) = series_cache.get(&series_info.series_id) {
                    let sequence = series_info.sequence.as_deref().unwrap_or("0");
                    let index = parse_series_index(sequence);

                    sqlx::query(
                        r#"
                        INSERT INTO SeriesBooks (series_id, book_id, "order", "index")
                        VALUES (?, ?, ?, ?)
                        "#,
                    )
                    .bind(series_id)
                    .bind(book_id)
                    .bind(sequence)
                    .bind(index)
                    .execute(pool)
                    .await?;
                }
            }
        }

        Ok(())
    }

    /// Update user-defined item (user-specific metadata)
    ///
    /// # Reference
    /// Based on `BookImporter.updateBook()` - DtoImporterService/BookImporter.cs:162-177
    async fn update_user_defined_item(
        &self,
        db: &Database,
        book_id: i64,
        item: &LibraryItem,
    ) -> Result<()> {
        let pool = db.pool();

        // Check if UserDefinedItem exists
        let exists: Option<(i64,)> =
            sqlx::query_as("SELECT book_id FROM UserDefinedItems WHERE book_id = ?")
                .bind(book_id)
                .fetch_optional(pool)
                .await?;

        if exists.is_none() {
            // Create new UserDefinedItem
            sqlx::query(
                r#"
                INSERT INTO UserDefinedItems (
                    book_id, tags, user_rating_overall, user_rating_performance, user_rating_story,
                    book_status, pdf_status, is_finished
                )
                VALUES (?, '', 0, 0, 0, 0, NULL, ?)
                "#,
            )
            .bind(book_id)
            .bind(item.is_finished.unwrap_or(false))
            .execute(pool)
            .await?;
        } else {
            // Update user ratings and is_finished
            let user_rating_overall = item.my_user_rating_overall.unwrap_or(0) as f32;
            let user_rating_performance = item.my_user_rating_performance.unwrap_or(0) as f32;
            let user_rating_story = item.my_user_rating_story.unwrap_or(0) as f32;
            let is_finished = item.is_finished.unwrap_or(false);

            sqlx::query(
                r#"
                UPDATE UserDefinedItems
                SET user_rating_overall = ?, user_rating_performance = ?, user_rating_story = ?, is_finished = ?
                WHERE book_id = ?
                "#
            )
            .bind(user_rating_overall)
            .bind(user_rating_performance)
            .bind(user_rating_story)
            .bind(is_finished)
            .bind(book_id)
            .execute(pool)
            .await?;
        }

        // Handle PDF supplement
        if let Some(ref pdf_url) = item.pdf_url {
            self.upsert_supplement(db, book_id, pdf_url).await?;
        }

        Ok(())
    }

    /// Upsert contributor
    async fn upsert_contributor(
        &self,
        db: &Database,
        name: &str,
        asin: Option<&str>,
    ) -> Result<i64> {
        let pool = db.pool();

        // Check if exists
        let existing: Option<(i64,)> =
            sqlx::query_as("SELECT contributor_id FROM Contributors WHERE name = ?")
                .bind(name)
                .fetch_optional(pool)
                .await?;

        match existing {
            Some((id,)) => Ok(id),
            None => {
                let result = sqlx::query(
                    "INSERT INTO Contributors (name, audible_contributor_id) VALUES (?, ?)",
                )
                .bind(name)
                .bind(asin)
                .execute(pool)
                .await?;

                Ok(result.last_insert_rowid())
            }
        }
    }

    /// Upsert series
    async fn upsert_series(
        &self,
        db: &Database,
        series_id: &str,
        name: Option<&str>,
    ) -> Result<i64> {
        let pool = db.pool();

        // Check if exists
        let existing: Option<(i64,)> =
            sqlx::query_as("SELECT series_id FROM Series WHERE audible_series_id = ?")
                .bind(series_id)
                .fetch_optional(pool)
                .await?;

        match existing {
            Some((id,)) => {
                // Update name if provided
                if let Some(name) = name {
                    sqlx::query("UPDATE Series SET name = ? WHERE series_id = ?")
                        .bind(name)
                        .bind(id)
                        .execute(pool)
                        .await?;
                }
                Ok(id)
            }
            None => {
                let result =
                    sqlx::query("INSERT INTO Series (audible_series_id, name) VALUES (?, ?)")
                        .bind(series_id)
                        .bind(name)
                        .execute(pool)
                        .await?;

                Ok(result.last_insert_rowid())
            }
        }
    }

    /// Upsert supplement (PDF)
    async fn upsert_supplement(&self, db: &Database, book_id: i64, url: &str) -> Result<()> {
        let pool = db.pool();

        // Check if exists
        let existing: Option<(i64,)> =
            sqlx::query_as("SELECT supplement_id FROM Supplements WHERE book_id = ?")
                .bind(book_id)
                .fetch_optional(pool)
                .await?;

        match existing {
            Some((id,)) => {
                sqlx::query("UPDATE Supplements SET url = ? WHERE supplement_id = ?")
                    .bind(url)
                    .bind(id)
                    .execute(pool)
                    .await?;
            }
            None => {
                sqlx::query("INSERT INTO Supplements (book_id, url) VALUES (?, ?)")
                    .bind(book_id)
                    .bind(url)
                    .execute(pool)
                    .await?;
            }
        }

        Ok(())
    }

    /// Mark books absent from last scan
    ///
    /// # Reference
    /// Based on `LibraryBookImporter.upsertLibraryBooks()` - DtoImporterService/LibraryBookImporter.cs:89-94
    async fn mark_absent_books(
        &self,
        db: &Database,
        items: &[LibraryItem],
        account_id: &str,
    ) -> Result<i32> {
        let pool = db.pool();

        // Get all ASINs from current sync, including podcast children referenced by parents.
        let mut current_asins: HashSet<String> = HashSet::new();
        for item in items {
            current_asins.insert(item.asin.clone());

            if let Some(relationships) = &item.relationships {
                for relationship in relationships {
                    if relationship
                        .relationship_to_product
                        .as_deref()
                        .map(|value| value.eq_ignore_ascii_case("child"))
                        .unwrap_or(false)
                    {
                        current_asins.insert(relationship.asin.clone());
                    }
                }
            }
        }

        // Get all ASINs in database for this account
        let db_books: Vec<(i64, String)> = sqlx::query_as(
            r#"
            SELECT b.book_id, b.audible_product_id
            FROM Books b
            INNER JOIN BookAccounts ba ON ba.book_id = b.book_id
            WHERE ba.account = ? AND ba.is_deleted = 0
            "#,
        )
        .bind(account_id)
        .fetch_all(pool)
        .await?;

        // Mark books absent that are not in current sync
        let mut absent_count = 0;
        for (book_id, asin) in db_books {
            if !current_asins.contains(&asin) {
                sqlx::query("UPDATE BookAccounts SET absent_from_last_scan = 1 WHERE book_id = ? AND account = ?")
                    .bind(book_id)
                    .bind(account_id)
                    .execute(pool)
                    .await?;

                absent_count += 1;
            }
        }

        Ok(absent_count)
    }
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Parse series index from order string
///
/// Converts series order strings like "1", "2.5", "Book 3" to numeric index.
/// Falls back to 0.0 if parsing fails.
fn parse_series_index(order: &str) -> f32 {
    // Try to extract first number from string
    let numbers: String = order
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    numbers.parse::<f32>().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_series_index() {
        assert_eq!(parse_series_index("1"), 1.0);
        assert_eq!(parse_series_index("2.5"), 2.5);
        assert_eq!(parse_series_index("Book 3"), 3.0);
        assert_eq!(parse_series_index("10"), 10.0);
        assert_eq!(parse_series_index("invalid"), 0.0);
    }

    #[test]
    fn take_raw_items_tolerates_a_missing_or_odd_items_field() {
        let mut with_items = serde_json::json!({"items": [{"asin": "A"}, {"asin": "B"}]});
        assert_eq!(AudibleClient::take_raw_items(&mut with_items).len(), 2);

        let mut empty = serde_json::json!({"items": [], "response_groups": "media"});
        assert!(AudibleClient::take_raw_items(&mut empty).is_empty());

        let mut missing = serde_json::json!({"response_groups": "media"});
        assert!(AudibleClient::take_raw_items(&mut missing).is_empty());

        let mut wrong_type = serde_json::json!({"items": "nope"});
        assert!(AudibleClient::take_raw_items(&mut wrong_type).is_empty());
    }

    #[test]
    fn items_survive_the_shapes_real_accounts_actually_return() {
        // Each of these cost a book (and, before per-item parsing, a whole page) because
        // the field was required. All are shapes Audible has really returned:
        // Libation had to relax the same validations for actual users.
        let cases = [
            // no purchase_date at all
            serde_json::json!({"asin": "A1", "title": "No date"}),
            // beginning-of-time purchase_date (Libation #1378)
            serde_json::json!({"asin": "A2", "title": "Epoch", "purchase_date": "0001-01-01T00:00:00Z"}),
            // a contributor with no name
            serde_json::json!({"asin": "A3", "title": "Nameless author",
                "purchase_date": "2024-01-01T00:00:00Z", "authors": [{"asin": "B1"}]}),
            // a series with no asin
            serde_json::json!({"asin": "A4", "title": "Series without id",
                "purchase_date": "2024-01-01T00:00:00Z", "series": [{"title": "Some series"}]}),
            // a relationship with no asin
            serde_json::json!({"asin": "A5", "title": "Odd relationship",
                "purchase_date": "2024-01-01T00:00:00Z",
                "relationships": [{"relationship_to_product": "child"}]}),
        ];

        for case in cases {
            let asin = case["asin"].as_str().unwrap().to_string();
            let item: LibraryItem = serde_json::from_value(case)
                .unwrap_or_else(|e| panic!("{asin} should still import: {e}"));
            assert_eq!(item.asin, asin);
        }
    }

    #[test]
    fn one_unparseable_item_does_not_cost_its_neighbours() {
        // A page used to be deserialized as a unit, so a single unreadable item took the
        // whole page — and, because an empty/failed page ends pagination, everything
        // after it too. (An unparseable date is the bad item here; a *missing* one is
        // tolerated, see items_survive_the_shapes_real_accounts_actually_return.)
        let mut page = serde_json::json!({"items": [
            {"asin": "GOOD1", "title": "First", "purchase_date": "2024-01-01T00:00:00Z"},
            {"asin": "BAD", "title": "Unreadable date", "purchase_date": "not-a-date"},
            {"asin": "GOOD2", "title": "Second", "purchase_date": "2024-02-01T00:00:00Z"}
        ]});

        let raw_items = AudibleClient::take_raw_items(&mut page);
        assert_eq!(raw_items.len(), 3);

        let mut parsed = Vec::new();
        let mut failed = Vec::new();
        for raw_item in raw_items {
            let asin = raw_item
                .get("asin")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            match serde_json::from_value::<LibraryItem>(raw_item) {
                Ok(item) => parsed.push(item),
                Err(_) => failed.push(asin),
            }
        }

        assert_eq!(parsed.len(), 2);
        assert_eq!(failed, vec!["BAD".to_string()]);
        assert_eq!(parsed[0].asin, "GOOD1");
        assert_eq!(parsed[1].asin, "GOOD2");
    }

    #[test]
    fn test_library_options_default() {
        let options = LibraryOptions::default();
        // Large pages on purpose: fewer requests per sync means fewer chances for a
        // transient failure to truncate the library. Audible allows up to 1000.
        assert_eq!(options.number_of_results_per_page, 250);
        assert!(options.number_of_results_per_page <= 1000);
        assert_eq!(options.page_number, 1);
        assert!(options.response_groups.contains("media"));
        assert!(options.response_groups.contains("contributors"));
    }

    #[test]
    fn test_library_item_null_plans() {
        // API may return "plans": null — must not fail deserialization
        let json = r#"{
            "asin": "B001TEST",
            "title": "Test Book",
            "purchase_date": "2024-01-01T00:00:00Z",
            "plans": null
        }"#;
        let item: LibraryItem = serde_json::from_str(json).unwrap();
        assert_eq!(item.asin, "B001TEST");
        assert!(item.plans.is_none());
    }

    #[test]
    fn test_library_item_missing_plans() {
        // plans field missing entirely should also work
        let json = r#"{
            "asin": "B002TEST",
            "title": "Test Book 2",
            "purchase_date": "2024-01-01T00:00:00Z"
        }"#;
        let item: LibraryItem = serde_json::from_str(json).unwrap();
        assert_eq!(item.asin, "B002TEST");
        assert!(item.plans.is_none());
    }

    #[test]
    fn test_library_item_nullable_collections() {
        let json = r#"{
            "items": [{
                "asin": "B08KBQQD15",
                "title": "Nullable Collections",
                "purchase_date": "2024-01-01T00:00:00Z",
                "available_codecs": null,
                "asset_details": null,
                "authors": null,
                "narrators": null,
                "category_ladders": null,
                "product_images": null
            }]
        }"#;

        let response: LibraryResponse = serde_json::from_str(json).unwrap();
        let item = &response.items[0];
        assert_eq!(item.asin, "B08KBQQD15");
        assert!(item.available_codecs.is_empty());
        assert!(item.asset_details.is_empty());
        assert!(item.authors.is_empty());
        assert!(item.narrators.is_empty());
        assert!(item.category_ladders.is_empty());
        assert!(item.product_images.is_empty());
    }

    #[test]
    fn test_library_item_with_plans() {
        let json = r#"{
            "asin": "B003TEST",
            "title": "Test Book 3",
            "purchase_date": "2024-01-01T00:00:00Z",
            "plans": [{"plan_name": "Premium Plus"}]
        }"#;
        let item: LibraryItem = serde_json::from_str(json).unwrap();
        assert!(item.plans.is_some());
        assert_eq!(item.plans.unwrap().len(), 1);
    }
}
