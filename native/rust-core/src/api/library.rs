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

use crate::error::{LibationError, Result};
use crate::api::client::AudibleClient;
use crate::api::auth::Account;
use crate::storage::Database;
use crate::storage::models::{
    Book, NewBook, NewLibraryBook, NewContributor, NewSeries, NewCategory, NewCategoryLadder,
    ContentType, Role, LibraryBook,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use chrono::{DateTime, NaiveDate, Utc};

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
            number_of_results_per_page: 50,  // Back to normal size
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
            ].join(","),
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
    #[serde(default)]
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
    #[serde(rename = "purchase_date")]
    pub purchase_date: DateTime<Utc>,

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
    /// Product description/summary
    #[serde(rename = "merchandising_summary", default)]
    pub description: Option<String>,

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
    #[serde(rename = "available_codecs", default)]
    pub available_codecs: Vec<CodecInfo>,

    /// Asset details (includes is_spatial for Dolby Atmos)
    #[serde(default)]
    pub asset_details: Vec<AssetDetail>,

    // === CONTRIBUTORS ===
    /// Authors
    #[serde(default)]
    pub authors: Vec<Person>,

    /// Narrators
    #[serde(default)]
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
    #[serde(rename = "category_ladders", default)]
    pub category_ladders: Vec<CategoryLadder>,

    // === IMAGES ===
    /// Product images at various sizes
    #[serde(rename = "product_images", default)]
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

    /// Subscription plans
    #[serde(default)]
    pub plans: Vec<Plan>,

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

    /// Get picture ID (highest quality image)
    /// Reference: BookImporter.cs:156-160
    pub fn get_picture_id(&self) -> Option<String> {
        // Try to get largest image (1215, then 500)
        self.product_images.get("1215")
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
        self.asset_details.iter().any(|a| a.is_spatial.unwrap_or(false))
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
    /// Person's name
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
    #[serde(rename = "asin")]
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
    #[serde(default)]
    pub ladder: Vec<CategoryNode>,
}

/// Category node in ladder
#[derive(Debug, Clone, Deserialize)]
pub struct CategoryNode {
    /// Category ID
    #[serde(rename = "id")]
    pub category_id: String,

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
    /// Related product ASIN
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
    pub async fn sync_library(
        &mut self,
        db: &Database,
        account: &Account,
    ) -> Result<SyncStats> {
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
        let (new_count, updated_count, errors) = self.import_items_to_db(db, &items, &account.account_id).await?;

        stats.books_added = new_count;
        stats.books_updated = updated_count;
        stats.errors = errors;

        // Mark absent books (removed from library)
        let absent_count = self.mark_absent_books(db, &items, &account.account_id).await?;
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

        let response: LibraryResponse = self
            .get_with_query("/1.0/library", &options)
            .await?;

        stats.total_items = response.items.len() as i32;

        // Set total_library_count and has_more from API response
        if let Some(total) = response.total_results {
            stats.total_library_count = total;
            let page_size = options.number_of_results_per_page;
            let total_pages = (total as f32 / page_size as f32).ceil() as i32;
            stats.has_more = page < total_pages;
        } else {
            // If no total provided, check if page is empty to determine has_more
            stats.has_more = !response.items.is_empty();
        }

        if response.items.is_empty() {
            return Ok(stats);
        }

        // Import items into database
        let (new_count, updated_count, errors) =
            self.import_items_to_db(db, &response.items, &account.account_id).await?;

        stats.books_added = new_count;
        stats.books_updated = updated_count;
        stats.errors = errors;

        // Note: books_absent is only calculated at the end of full sync
        // Individual pages don't mark absent books

        Ok(stats)
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
        let first_response: LibraryResponse = self
            .get_with_query("/1.0/library", &options)
            .await?;

        all_items.extend(first_response.items);

        // If API provides total_results, use it for pagination
        if let Some(total) = first_response.total_results {
            let page_size = options.number_of_results_per_page;
            let total_pages = (total as f32 / page_size as f32).ceil() as i32;

            // Fetch remaining pages
            for page_num in 2..=total_pages {
                options.page_number = page_num;
                let response: LibraryResponse = self
                    .get_with_query("/1.0/library", &options)
                    .await?;

                all_items.extend(response.items);
            }

            Ok((all_items, total))
        } else {
            // API doesn't provide total - keep fetching until empty response
            let page_size = options.number_of_results_per_page;
            let mut page_num = 2;

            loop {
                options.page_number = page_num;
                let response: LibraryResponse = self
                    .get_with_query("/1.0/library", &options)
                    .await?;

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

        // Build lookup maps for contributors, series, categories
        let mut contributor_cache: HashMap<String, i64> = HashMap::new();
        let mut series_cache: HashMap<String, i64> = HashMap::new();
        let mut category_cache: HashMap<String, i64> = HashMap::new();

        // Import contributors first (authors, narrators, publishers)
        for item in items {
            for author in &item.authors {
                if !contributor_cache.contains_key(&author.name) {
                    match self.upsert_contributor(db, &author.name, author.asin.as_deref()).await {
                        Ok(id) => { contributor_cache.insert(author.name.clone(), id); },
                        Err(e) => errors.push(format!("Failed to import author '{}': {}", author.name, e)),
                    }
                }
            }

            for narrator in &item.narrators {
                if !contributor_cache.contains_key(&narrator.name) {
                    match self.upsert_contributor(db, &narrator.name, narrator.asin.as_deref()).await {
                        Ok(id) => { contributor_cache.insert(narrator.name.clone(), id); },
                        Err(e) => errors.push(format!("Failed to import narrator '{}': {}", narrator.name, e)),
                    }
                }
            }

            if let Some(ref publisher) = item.publisher {
                if !contributor_cache.contains_key(publisher) {
                    match self.upsert_contributor(db, publisher, None).await {
                        Ok(id) => { contributor_cache.insert(publisher.clone(), id); },
                        Err(e) => errors.push(format!("Failed to import publisher '{}': {}", publisher, e)),
                    }
                }
            }
        }

        // Import series
        for item in items {
            if let Some(series_list) = &item.series {
                for series_info in series_list {
                    if !series_cache.contains_key(&series_info.series_id) {
                        match self.upsert_series(db, &series_info.series_id, series_info.title.as_deref()).await {
                            Ok(id) => { series_cache.insert(series_info.series_id.clone(), id); },
                            Err(e) => errors.push(format!("Failed to import series '{}': {}", series_info.series_id, e)),
                        }
                    }
                }
            }
        }

        // Import books and link relationships
        for item in items {
            match self.import_book(db, item, account_id, &contributor_cache, &series_cache).await {
                Ok(is_new) => {
                    if is_new {
                        new_count += 1;
                    } else {
                        updated_count += 1;
                    }
                },
                Err(e) => {
                    errors.push(format!("Failed to import book '{}': {}", item.asin, e));
                }
            }
        }

        Ok((new_count, updated_count, errors))
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
        let existing: Option<(i64,)> = sqlx::query_as(
            "SELECT book_id FROM Books WHERE audible_product_id = ?"
        )
        .bind(&item.asin)
        .fetch_optional(pool)
        .await?;

        let (book_id, is_new) = match existing {
            Some((id,)) => {
                // Update existing book
                self.update_book(db, id, item).await?;
                (id, false)
            },
            None => {
                // Create new book
                let id = self.create_book(db, item).await?;
                (id, true)
            }
        };

        // Upsert LibraryBook record
        self.upsert_library_book(db, book_id, account_id, &item.purchase_date).await?;

        // Link contributors (authors, narrators, publisher)
        self.link_contributors(db, book_id, item, contributor_cache).await?;

        // Link series
        self.link_series(db, book_id, item, series_cache).await?;

        // Update user-defined metadata
        self.update_user_defined_item(db, book_id, item).await?;

        Ok(is_new)
    }

    /// Create new book record
    ///
    /// # Reference
    /// Based on `BookImporter.createNewBook()` - DtoImporterService/BookImporter.cs:74-144
    async fn create_book(&self, db: &Database, item: &LibraryItem) -> Result<i64> {
        let pool = db.pool();

        let content_type = item.get_content_type() as i32;
        let description = item.description.as_deref().unwrap_or("");
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
        let is_downloadable = item.is_downloadable.unwrap_or(true);
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
        let is_downloadable = item.is_downloadable.unwrap_or(true);
        let is_ayce = item.is_ayce.unwrap_or(false);
        let origin_asin = item.origin_asin.as_deref();
        let episode_number = item.episode_number;
        let content_delivery_type = item.content_delivery_type.as_deref();

        sqlx::query(
            r#"
            UPDATE Books
            SET title = ?, subtitle = ?, length_in_minutes = ?, is_abridged = ?, is_spatial = ?,
                date_published = ?, language = ?, picture_id = ?, picture_large = ?,
                rating_overall = ?, rating_performance = ?, rating_story = ?,
                pdf_url = ?, is_finished = ?, is_downloadable = ?, is_ayce = ?,
                origin_asin = ?, episode_number = ?, content_delivery_type = ?,
                updated_at = datetime('now')
            WHERE book_id = ?
            "#
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

        // Check if LibraryBook exists
        let exists: Option<(bool,)> = sqlx::query_as(
            "SELECT is_deleted FROM LibraryBooks WHERE book_id = ?"
        )
        .bind(book_id)
        .fetch_optional(pool)
        .await?;

        match exists {
            Some(_) => {
                // Update existing - mark as not absent, not deleted
                sqlx::query(
                    r#"
                    UPDATE LibraryBooks
                    SET account = ?, absent_from_last_scan = 0, is_deleted = 0
                    WHERE book_id = ?
                    "#
                )
                .bind(account_id)
                .bind(book_id)
                .execute(pool)
                .await?;
            },
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
                    "#
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
                    "#
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
                    "#
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
                        "#
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
        let exists: Option<(i64,)> = sqlx::query_as(
            "SELECT book_id FROM UserDefinedItems WHERE book_id = ?"
        )
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
                "#
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
    async fn upsert_contributor(&self, db: &Database, name: &str, asin: Option<&str>) -> Result<i64> {
        let pool = db.pool();

        // Check if exists
        let existing: Option<(i64,)> = sqlx::query_as(
            "SELECT contributor_id FROM Contributors WHERE name = ?"
        )
        .bind(name)
        .fetch_optional(pool)
        .await?;

        match existing {
            Some((id,)) => Ok(id),
            None => {
                let result = sqlx::query(
                    "INSERT INTO Contributors (name, audible_contributor_id) VALUES (?, ?)"
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
    async fn upsert_series(&self, db: &Database, series_id: &str, name: Option<&str>) -> Result<i64> {
        let pool = db.pool();

        // Check if exists
        let existing: Option<(i64,)> = sqlx::query_as(
            "SELECT series_id FROM Series WHERE audible_series_id = ?"
        )
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
            },
            None => {
                let result = sqlx::query(
                    "INSERT INTO Series (audible_series_id, name) VALUES (?, ?)"
                )
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
        let existing: Option<(i64,)> = sqlx::query_as(
            "SELECT supplement_id FROM Supplements WHERE book_id = ?"
        )
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
            },
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

        // Get all ASINs from current sync
        let current_asins: HashSet<String> = items.iter().map(|i| i.asin.clone()).collect();

        // Get all ASINs in database for this account
        let db_books: Vec<(i64, String)> = sqlx::query_as(
            r#"
            SELECT b.book_id, b.audible_product_id
            FROM Books b
            INNER JOIN LibraryBooks lb ON lb.book_id = b.book_id
            WHERE lb.account = ? AND lb.is_deleted = 0
            "#
        )
        .bind(account_id)
        .fetch_all(pool)
        .await?;

        // Mark books absent that are not in current sync
        let mut absent_count = 0;
        for (book_id, asin) in db_books {
            if !current_asins.contains(&asin) {
                sqlx::query("UPDATE LibraryBooks SET absent_from_last_scan = 1 WHERE book_id = ?")
                    .bind(book_id)
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
    let numbers: String = order.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
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
    fn test_library_options_default() {
        let options = LibraryOptions::default();
        assert_eq!(options.number_of_results_per_page, 50);
        assert_eq!(options.page_number, 1);
        assert!(options.response_groups.contains("media"));
        assert!(options.response_groups.contains("contributors"));
    }
}
