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


//! Content metadata and catalog queries
//!
//! # Reference C# Sources
//! - **External: `AudibleApi/Api.cs`** - GetCatalogProductAsync(asin, responseGroups), GetCatalogProductsAsync(asins)
//! - **External: `AudibleApi/Common/ContentMetadata.cs`** - ContentMetadata, ChapterInfo, Chapter structures
//! - **External: `AudibleApi/Common/Item.cs`** - CatalogProduct (richer than LibraryItem)
//! - **`FileLiberator/AudioDecodable.cs`** - Content URL resolution
//! - **`AaxDecrypter/AudiobookDownloadBase.cs`** - Content metadata for downloads
//! - **`AudibleUtilities/ApiExtended.cs`** - GetCatalogProductsAsync batch query (lines 200-223)
//!
//! # Key Functionality
//! - Fetch detailed product information by ASIN
//! - Get chapter/section information with timestamps
//! - Query available quality tiers and codecs
//! - Get content URLs (download, streaming)
//! - Batch product queries for episodes/series
//!
//! # Catalog API Endpoints
//!
//! ## Single Product Query
//! **GET** `/1.0/catalog/products/{asin}`
//!
//! Query parameters:
//! - `response_groups` - Comma-separated list:
//!   - `product_desc` - Description, publisher, release date
//!   - `product_attrs` - Runtime, language, ASIN
//!   - `media` - Available formats and URLs
//!   - `relationships` - Series, episodes, parent/child
//!   - `rating` - Customer ratings and review count
//!   - `contributors` - Authors, narrators
//!   - `series` - Series information
//!   - `product_extended_attrs` - Extended attributes
//!   - `product_plans` - Subscription plan availability
//!   - `provided_review` - User's own review
//! - `image_sizes` - Comma-separated sizes (e.g., "500,1024")
//!
//! ## Batch Product Query
//! **GET** `/1.0/catalog/products`
//!
//! Query parameters:
//! - `asin` - Comma-separated list of ASINs (max 50 per request)
//! - `response_groups` - Same as single product
//! - `image_sizes` - Same as single product
//!
//! Reference: ApiExtended.cs:206 - Uses CatalogOptions.ResponseGroupOptions for batch queries
//!
//! # Content Metadata Endpoint
//! **GET** `/1.0/content/{asin}/metadata`
//!
//! Returns:
//! - Chapter information (title, start offset, duration)
//! - Content reference (ACR, SKU, version, codec)
//! - Brand intro/outro durations (Audible branding audio)
//! - Runtime length
//!
//! Reference: DownloadOptions.Factory.cs:33 - api.GetContentMetadataAsync()

use crate::error::{LibationError, Result};
use crate::api::client::AudibleClient;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc, NaiveDate};

// ============================================================================
// CORE TYPES - DRM and Codecs
// ============================================================================

/// DRM type for audiobook content
/// Reference: AudibleApi.Common.DrmType, DownloadOptions.cs:40, DownloadOptions.cs:69-76
///
/// C# enum values:
/// - `Adrm` - Audible DRM (AAX and AAXC formats)
/// - `Widevine` - Widevine DRM (MPEG-DASH format)
/// - Other values indicate unencrypted content
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DrmType {
    /// Audible DRM (AAX/AAXC format)
    /// - AAX: Legacy format with 4-byte activation bytes
    /// - AAXC: Current format with 16-byte key pairs
    #[serde(rename = "Adrm")]
    Adrm,

    /// Widevine DRM (MPEG-DASH format)
    /// Requires Widevine CDM for decryption
    #[serde(rename = "Mpeg")]
    Widevine,

    /// No DRM (unencrypted MP3 or M4B)
    /// Some older audiobooks or podcasts
    #[serde(rename = "None")]
    None,
}

impl DrmType {
    /// Check if content is encrypted
    pub fn is_encrypted(&self) -> bool {
        matches!(self, DrmType::Adrm | DrmType::Widevine)
    }

    /// Check if content requires activation bytes (AAX)
    pub fn requires_activation_bytes(&self) -> bool {
        matches!(self, DrmType::Adrm)
    }

    /// Check if content uses Widevine CDM
    pub fn is_widevine(&self) -> bool {
        matches!(self, DrmType::Widevine)
    }
}

/// Audio codec types
/// Reference: AudibleApi.Codecs, DownloadOptions.Factory.cs:72-74
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Codec {
    /// AAC Low Complexity (standard AAC)
    #[serde(rename = "AAC_LC")]
    AacLc,

    /// Extended HE-AAC (high efficiency)
    #[serde(rename = "xHE_AAC")]
    XHeAac,

    /// Enhanced AC-3 (spatial audio)
    #[serde(rename = "EC_3")]
    Ec3,

    /// AC-4 (advanced spatial audio)
    /// Reference: DownloadOptions.Factory.cs:77 - AC_4 check for lossy conversion
    #[serde(rename = "AC_4")]
    Ac4,

    /// MP3 (legacy format)
    #[serde(rename = "MP3")]
    Mp3,
}

/// Download quality tiers
/// Reference: ApiExtended.cs batch query, DownloadOptions.Factory.cs:59
///
/// C# enum: DownloadQuality (Normal, High, Extreme)
/// API values: "Normal", "High", "Extreme"
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DownloadQuality {
    /// Low quality (~32 kbps AAC)
    #[serde(rename = "Low")]
    Low,

    /// Normal quality (~64 kbps AAC)
    #[serde(rename = "Normal")]
    Normal,

    /// High quality (~128 kbps AAC)
    #[serde(rename = "High")]
    High,

    /// Extreme quality (highest available)
    /// May include spatial audio formats
    #[serde(rename = "Extreme")]
    Extreme,
}

/// Chapter title nesting type
/// Reference: DownloadOptions.Factory.cs:80 - ChapterTitlesType.Tree
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChapterTitlesType {
    /// Flat chapter list (no nesting)
    #[serde(rename = "Flat")]
    Flat,

    /// Tree structure (chapters can have sub-chapters)
    /// Reference: DownloadOptions.Factory.cs:182-289 - flattenChapters() function
    #[serde(rename = "Tree")]
    Tree,
}

// ============================================================================
// CONTENT STRUCTURES
// ============================================================================

/// Chapter information with timing
/// Reference: AudibleApi.Common.Chapter, DownloadOptions.Factory.cs:257-289
///
/// C# properties:
/// - Title (string)
/// - StartOffsetMs (long)
/// - StartOffsetSec (int)
/// - LengthMs (long)
/// - Chapters (List<Chapter>?) - For hierarchical chapters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
    /// Chapter title
    pub title: String,

    /// Start offset in milliseconds from beginning of audiobook
    #[serde(rename = "start_offset_ms")]
    pub start_offset_ms: i64,

    /// Start offset in seconds (convenience field)
    #[serde(rename = "start_offset_sec")]
    pub start_offset_sec: i32,

    /// Chapter duration in milliseconds
    #[serde(rename = "length_ms")]
    pub length_ms: i64,

    /// Nested chapters (for hierarchical structure)
    /// Reference: DownloadOptions.Factory.cs:182-289 - flattenChapters handles nesting
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chapters: Option<Vec<Chapter>>,
}

/// Chapter information container
/// Reference: AudibleApi.Common.ChapterInfo, DownloadOptions.cs:22
///
/// C# properties:
/// - BrandIntroDurationMs (int)
/// - BrandOutroDurationMs (int)
/// - Chapters (List<Chapter>)
/// - IsAccurate (bool)
/// - RuntimeLengthMs (long)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterInfo {
    /// Audible brand intro duration (in milliseconds)
    /// Can be stripped if configured
    #[serde(rename = "brandIntroDurationMs", default)]
    pub brand_intro_duration_ms: i32,

    /// Audible brand outro duration (in milliseconds)
    /// Can be stripped if configured
    #[serde(rename = "brandOutroDurationMs", default)]
    pub brand_outro_duration_ms: i32,

    /// List of chapters
    #[serde(rename = "chapters", default)]
    pub chapters: Vec<Chapter>,

    /// Whether chapter timing is accurate
    /// Sometimes Audible provides inaccurate chapter metadata
    /// Reference: DownloadOptions.Factory.cs:29-35 - metadata comparison
    #[serde(rename = "isAccurate", default)]
    pub is_accurate: bool,

    /// Total runtime in milliseconds
    #[serde(rename = "runtimeLengthMs")]
    pub runtime_length_ms: i64,
}

/// Content reference with DRM information
/// Reference: AudibleApi.Common.ContentReference, DownloadOptions.cs:41
///
/// C# properties:
/// - Acr (string) - Audible Content Reference
/// - Sku (string) - Stock Keeping Unit
/// - Version (string) - Content version
/// - Codec (Codec) - Audio codec
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentReference {
    /// Audible Content Reference (unique content identifier)
    #[serde(rename = "acr")]
    pub acr: String,

    /// Stock Keeping Unit
    #[serde(rename = "sku")]
    pub sku: String,

    /// Content version (for tracking updates)
    #[serde(rename = "version")]
    pub version: String,

    /// Audio codec
    #[serde(rename = "codec")]
    pub codec: Codec,
}

/// Content URL information
/// Reference: DownloadOptions.cs:61-62 - ContentMetadata.ContentUrl.OfflineUrl
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentUrl {
    /// URL for offline download
    /// This is the CDN URL for downloading the encrypted audiobook file
    #[serde(rename = "offline_url")]
    pub offline_url: Option<String>,

    /// URL for streaming playback
    #[serde(rename = "streaming_url")]
    pub streaming_url: Option<String>,
}

/// Complete content metadata
/// Reference: AudibleApi.Common.ContentMetadata, DownloadOptions.cs:41
///
/// C# properties:
/// - ChapterInfo (ChapterInfo)
/// - ContentReference (ContentReference)
/// - ContentUrl (ContentUrl)
///
/// Note: Different API endpoints return different subsets:
/// - /1.0/content/{asin}/metadata - Returns all fields
/// - /1.0/content/{asin}/licenserequest - Returns only content_url
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentMetadata {
    /// Chapter information with timing
    /// Only present in full metadata endpoint, not in license response
    #[serde(rename = "chapter_info", skip_serializing_if = "Option::is_none")]
    pub chapter_info: Option<ChapterInfo>,

    /// Content reference with codec/version info
    /// Only present in full metadata endpoint, not in license response
    #[serde(rename = "content_reference", skip_serializing_if = "Option::is_none")]
    pub content_reference: Option<ContentReference>,

    /// Download and streaming URLs
    #[serde(rename = "content_url")]
    pub content_url: ContentUrl,
}

// ============================================================================
// CATALOG PRODUCT (Richer than LibraryItem)
// ============================================================================

/// Contributor information (author, narrator, etc.)
/// Reference: AudibleApi.Common.Contributor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contributor {
    /// Contributor's unique ID
    #[serde(rename = "asin")]
    pub asin: String,

    /// Display name
    #[serde(rename = "name")]
    pub name: String,

    /// Role (author, narrator, etc.)
    #[serde(rename = "role")]
    pub role: String,
}

/// Product rating information
/// Reference: AudibleApi.Common.Rating
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rating {
    /// Overall rating (0.0 - 5.0)
    #[serde(rename = "overall_distribution")]
    pub overall_distribution: Option<RatingDistribution>,

    /// Number of reviews
    #[serde(rename = "num_reviews")]
    pub num_reviews: i32,
}

/// Rating distribution details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RatingDistribution {
    /// Average rating
    #[serde(rename = "average_rating")]
    pub average_rating: f32,

    /// Display stars (formatted string like "4.5 out of 5 stars")
    #[serde(rename = "display_stars")]
    pub display_stars: Option<String>,
}

/// Series information
/// Reference: AudibleApi.Common.Series
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Series {
    /// Series ASIN
    #[serde(rename = "asin")]
    pub asin: String,

    /// Series title
    #[serde(rename = "title")]
    pub title: String,

    /// Book's position in series (e.g., "1", "2.5")
    #[serde(rename = "sequence")]
    pub sequence: Option<String>,
}

/// Relationship to other products (series, episodes)
/// Reference: AudibleApi.Common.Relationship, ApiExtended.cs:106-116
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    /// Related product ASIN
    #[serde(rename = "asin")]
    pub asin: String,

    /// Relationship type (e.g., "episode", "season", "series")
    /// Reference: ApiExtended.cs:112 - RelationshipType.Episode
    #[serde(rename = "relationship_type")]
    pub relationship_type: String,

    /// Relationship to product (e.g., "parent", "child")
    /// Reference: ApiExtended.cs:107, 111 - RelationshipToProduct enum
    #[serde(rename = "relationship_to_product")]
    pub relationship_to_product: String,

    /// Sort order for episodes
    /// Reference: ApiExtended.cs:259 - parent.Relationships[].Sort
    #[serde(rename = "sort")]
    pub sort: Option<i32>,
}

/// Detailed product information from catalog
/// Reference: AudibleApi.Common.Item (CatalogProduct), ApiExtended.cs:206-210
///
/// This is richer than LibraryItem and includes:
/// - Full product description
/// - Publisher information
/// - Detailed ratings
/// - Editorial reviews
/// - Series relationships
/// - Episode information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogProduct {
    /// Audible product ID (ASIN)
    #[serde(rename = "asin")]
    pub asin: String,

    /// Product title
    #[serde(rename = "title")]
    pub title: String,

    /// Subtitle (if any)
    #[serde(rename = "subtitle")]
    pub subtitle: Option<String>,

    /// Full product description/summary
    #[serde(rename = "publisher_summary")]
    pub publisher_summary: Option<String>,

    /// Publisher name
    #[serde(rename = "publisher_name")]
    pub publisher_name: Option<String>,

    /// Release date
    #[serde(rename = "release_date")]
    pub release_date: Option<NaiveDate>,

    /// Runtime in minutes
    #[serde(rename = "runtime_length_min")]
    pub runtime_length_min: i32,

    /// Content language (e.g., "english", "spanish")
    #[serde(rename = "language")]
    pub language: String,

    /// Format type (e.g., "unabridged", "abridged")
    #[serde(rename = "format_type")]
    pub format_type: String,

    /// Authors, narrators, etc.
    #[serde(rename = "authors")]
    pub authors: Vec<Contributor>,

    /// Narrators
    #[serde(rename = "narrators")]
    pub narrators: Vec<Contributor>,

    /// Customer ratings
    #[serde(rename = "rating")]
    pub rating: Option<Rating>,

    /// Series information
    #[serde(rename = "series")]
    pub series: Vec<Series>,

    /// Relationships to other products
    /// Used for episodes/series parent-child relationships
    /// Reference: ApiExtended.cs:106-116 - Episode and parent ASIN extraction
    #[serde(rename = "relationships")]
    pub relationships: Vec<Relationship>,

    /// Product URL (cover image)
    #[serde(rename = "product_images")]
    pub product_images: Option<HashMap<String, String>>,

    /// Episode number (for podcast episodes)
    /// Reference: ApiExtended.cs:257 - child.EpisodeNumber
    #[serde(rename = "episode_number")]
    pub episode_number: Option<i32>,

    /// Publication date (for podcasts/episodes)
    /// Reference: ApiExtended.cs:253 - OrderBy(i => i.PublicationDateTime)
    #[serde(rename = "publication_datetime")]
    pub publication_datetime: Option<DateTime<Utc>>,

    /// Whether this is a series parent
    /// Reference: ApiExtended.cs:103, 145 - i.IsSeriesParent
    #[serde(rename = "is_series_parent")]
    pub is_series_parent: bool,

    /// Whether this is an episode (podcast episode or series child)
    /// Reference: ApiExtended.cs:102, 147 - i.IsEpisodes
    #[serde(rename = "is_episode")]
    pub is_episode: bool,
}

// ============================================================================
// API FUNCTIONS
// ============================================================================

impl AudibleClient {
    /// Get detailed product information by ASIN
    ///
    /// # Reference
    /// C# method: `Api.GetCatalogProductAsync(asin, responseGroups)`
    /// Location: AudibleApi/Api.cs (external package)
    ///
    /// # Endpoint
    /// `GET /1.0/catalog/products/{asin}`
    ///
    /// # Arguments
    /// * `asin` - Audible product ID
    ///
    /// # Returns
    /// Detailed catalog product information
    ///
    /// # Errors
    /// - `ApiRequestFailed` - API request failed (network, 4xx, 5xx errors)
    /// - `InvalidApiResponse` - Response parsing failed
    /// - `RecordNotFound` - Product not found (404)
    ///
    /// # Example
    /// ```rust,no_run
    /// # use rust_core::api::client::AudibleClient;
    /// # use rust_core::api::auth::Account;
    /// # async fn example() -> rust_core::error::Result<()> {
    /// let client = AudibleClient::new(Account::default())?;
    /// let product = client.get_catalog_product("B002V5D7B0").await?;
    /// println!("Title: {}", product.title);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_catalog_product(&self, asin: &str) -> Result<CatalogProduct> {
        let endpoint = format!("/1.0/catalog/products/{}", asin);

        // Include all response groups for complete product information
        // Reference: ApiExtended.cs:206-210 - CatalogOptions.ResponseGroupOptions
        let response_groups = vec![
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
        ].join(",");

        let params = vec![
            ("response_groups", response_groups),
            ("image_sizes", "500".to_string()),
        ];

        // Build query string
        let query_string = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        let url = format!("{}?{}", endpoint, query_string);

        // Make request using client's retry logic
        let response: serde_json::Value = self.get(&url).await?;

        // Parse response
        // The API wraps the product in a "product" field
        let product_json = response
            .get("product")
            .ok_or_else(|| LibationError::InvalidApiResponse {
                message: "Missing 'product' field in response".to_string(),
                response_body: Some(response.to_string()),
            })?;

        serde_json::from_value(product_json.clone())
            .map_err(|e| LibationError::InvalidApiResponse {
                message: format!("Failed to parse catalog product: {}", e),
                response_body: Some(product_json.to_string()),
            })
    }

    /// Get multiple products in a single batch request
    ///
    /// # Reference
    /// C# method: `Api.GetCatalogProductsAsync(asins, responseGroups)`
    /// Location: ApiExtended.cs:200-223 - getProductsAsync() for episode batching
    ///
    /// # Endpoint
    /// `GET /1.0/catalog/products?asin=A,B,C`
    ///
    /// # Arguments
    /// * `asins` - List of Audible product IDs (max 50 per request)
    ///
    /// # Returns
    /// Vector of catalog products (may be fewer than requested if some ASINs are invalid)
    ///
    /// # Errors
    /// - `ApiRequestFailed` - API request failed
    /// - `InvalidApiResponse` - Response parsing failed
    /// - `InvalidInput` - Too many ASINs (>50)
    ///
    /// # Example
    /// ```rust,no_run
    /// # use rust_core::api::client::AudibleClient;
    /// # use rust_core::api::auth::Account;
    /// # async fn example() -> rust_core::error::Result<()> {
    /// let client = AudibleClient::new(Account::default())?;
    /// let asins = vec!["B002V5D7B0".to_string(), "B002V1O97Y".to_string()];
    /// let products = client.get_catalog_products_batch(asins).await?;
    /// println!("Retrieved {} products", products.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_catalog_products_batch(&self, asins: Vec<String>) -> Result<Vec<CatalogProduct>> {
        // Validate batch size (max 50 per request)
        // Reference: ApiExtended.cs:24 - BatchSize = 50
        if asins.len() > crate::api::client::BATCH_SIZE {
            return Err(LibationError::invalid_input(
                format!("Too many ASINs in batch: {} (max {})", asins.len(), crate::api::client::BATCH_SIZE)
            ));
        }

        if asins.is_empty() {
            return Ok(Vec::new());
        }

        let endpoint = "/1.0/catalog/products";

        // Response groups for batch query
        // Reference: ApiExtended.cs:206-210
        let response_groups = vec![
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
        ].join(",");

        // Build query with comma-separated ASINs
        let asin_param = asins.join(",");
        let params = vec![
            ("asin", asin_param),
            ("response_groups", response_groups),
            ("image_sizes", "500".to_string()),
        ];

        let query_string = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        let url = format!("{}?{}", endpoint, query_string);

        let response: serde_json::Value = self.get(&url).await?;

        // Parse products array
        let products_json = response
            .get("products")
            .and_then(|p| p.as_array())
            .ok_or_else(|| LibationError::InvalidApiResponse {
                message: "Missing or invalid 'products' array in response".to_string(),
                response_body: Some(response.to_string()),
            })?;

        // Parse each product
        let mut products = Vec::with_capacity(products_json.len());
        for product_value in products_json {
            match serde_json::from_value(product_value.clone()) {
                Ok(product) => products.push(product),
                Err(e) => {
                    // Log parsing error but continue with other products
                    eprintln!("Warning: Failed to parse product in batch: {}", e);
                }
            }
        }

        Ok(products)
    }

    /// Get content metadata including chapter information
    ///
    /// # Reference
    /// C# method: `Api.GetContentMetadataAsync(asin)`
    /// Location: DownloadOptions.Factory.cs:33 - metadata endpoint for accurate chapters
    ///
    /// # Endpoint
    /// `GET /1.0/content/{asin}/metadata`
    ///
    /// # Arguments
    /// * `asin` - Audible product ID
    ///
    /// # Returns
    /// Content metadata with chapter timing and codec information
    ///
    /// # Errors
    /// - `ApiRequestFailed` - API request failed
    /// - `InvalidApiResponse` - Response parsing failed
    /// - `RecordNotFound` - Content metadata not found
    ///
    /// # Note
    /// This endpoint provides more accurate chapter information than the license request.
    /// Reference: DownloadOptions.Factory.cs:29-35 - Compares RuntimeLengthMs to verify accuracy
    ///
    /// # Example
    /// ```rust,no_run
    /// # use rust_core::api::client::AudibleClient;
    /// # use rust_core::api::auth::Account;
    /// # async fn example() -> rust_core::error::Result<()> {
    /// let client = AudibleClient::new(Account::default())?;
    /// let metadata = client.get_content_metadata("B002V5D7B0").await?;
    /// println!("Runtime: {} minutes", metadata.chapter_info.runtime_length_ms / 60000);
    /// println!("Chapters: {}", metadata.chapter_info.chapters.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_content_metadata(&self, asin: &str) -> Result<ContentMetadata> {
        let endpoint = format!("/1.0/content/{}/metadata", asin);

        let response: serde_json::Value = self.get(&endpoint).await?;

        // Parse metadata
        // The API may wrap in a "content_metadata" field or return directly
        let metadata_json = response
            .get("content_metadata")
            .unwrap_or(&response);

        serde_json::from_value(metadata_json.clone())
            .map_err(|e| LibationError::InvalidApiResponse {
                message: format!("Failed to parse content metadata: {}", e),
                response_body: Some(metadata_json.to_string()),
            })
    }
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Flatten hierarchical chapters into a flat list
///
/// # Reference
/// C# method: `DownloadOptions.flattenChapters(chapters, titleConcat)`
/// Location: DownloadOptions.Factory.cs:257-289
///
/// This function ports Libation's chapter flattening logic which combines nested
/// chapters (like "Book 1" -> "Part 1" -> "Chapter 1") into a flat list with
/// concatenated titles (e.g., "Book 1: Part 1: Chapter 1").
///
/// # Arguments
/// * `chapters` - Hierarchical chapter list
/// * `title_separator` - Separator for concatenating titles (e.g., ": ")
///                       If None, preserves hierarchy as separate chapters
///
/// # Returns
/// Flattened chapter list with concatenated titles
///
/// # Example
/// ```rust,ignore
/// let chapters = vec![/* nested chapters */];
/// let flat = flatten_chapters(chapters, Some(": "));
/// ```
pub fn flatten_chapters(chapters: Vec<Chapter>, title_separator: Option<&str>) -> Vec<Chapter> {
    let mut result = Vec::new();

    for chapter in chapters {
        if chapter.chapters.is_none() {
            // Leaf chapter - add directly
            result.push(chapter);
        } else if let Some(sep) = title_separator {
            // Has children and should concatenate titles
            let mut children = chapter.chapters.unwrap_or_default();

            // If parent chapter is short (<10 seconds), merge with first child
            // Reference: DownloadOptions.Factory.cs:272-278
            if chapter.length_ms < 10000 && !children.is_empty() {
                children[0].start_offset_ms = chapter.start_offset_ms;
                children[0].start_offset_sec = chapter.start_offset_sec;
                children[0].length_ms += chapter.length_ms;
            } else {
                // Parent is long enough - keep as separate chapter
                result.push(Chapter {
                    title: chapter.title.clone(),
                    start_offset_ms: chapter.start_offset_ms,
                    start_offset_sec: chapter.start_offset_sec,
                    length_ms: chapter.length_ms,
                    chapters: None,
                });
            }

            // Recursively flatten children and prepend parent title
            let flattened_children = flatten_chapters(children, Some(sep));
            for mut child in flattened_children {
                child.title = format!("{}{}{}", chapter.title, sep, child.title);
                result.push(child);
            }
        } else {
            // No separator - keep hierarchy
            result.push(chapter.clone());
            if let Some(children) = chapter.chapters {
                result.extend(flatten_chapters(children, None));
            }
        }
    }

    result
}

/// Combine opening and ending credits chapters
///
/// # Reference
/// C# method: `DownloadOptions.combineCredits(chapters)`
/// Location: DownloadOptions.Factory.cs:292-306
///
/// Merges "Opening Credits" and "End Credits" chapters into adjacent chapters
/// to avoid very short chapters that are just branding.
///
/// # Arguments
/// * `chapters` - Mutable chapter list
///
/// # Example
/// ```rust,ignore
/// let mut chapters = vec![/* chapters with credits */];
/// combine_credits(&mut chapters);
/// ```
pub fn combine_credits(chapters: &mut Vec<Chapter>) {
    // Combine "Opening Credits" with next chapter
    if chapters.len() > 1 && chapters[0].title == "Opening Credits" {
        chapters[1].start_offset_ms = chapters[0].start_offset_ms;
        chapters[1].start_offset_sec = chapters[0].start_offset_sec;
        chapters[1].length_ms += chapters[0].length_ms;
        chapters.remove(0);
    }

    // Combine "End Credits" with previous chapter
    if chapters.len() > 1 {
        let last_idx = chapters.len() - 1;
        if chapters[last_idx].title == "End Credits" {
            chapters[last_idx - 1].length_ms += chapters[last_idx].length_ms;
            chapters.remove(last_idx);
        }
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_drm_type_checks() {
        assert!(DrmType::Adrm.is_encrypted());
        assert!(DrmType::Widevine.is_encrypted());
        assert!(!DrmType::None.is_encrypted());

        assert!(DrmType::Adrm.requires_activation_bytes());
        assert!(!DrmType::Widevine.requires_activation_bytes());

        assert!(DrmType::Widevine.is_widevine());
        assert!(!DrmType::Adrm.is_widevine());
    }

    #[test]
    fn test_flatten_chapters_simple() {
        let chapters = vec![
            Chapter {
                title: "Chapter 1".to_string(),
                start_offset_ms: 0,
                start_offset_sec: 0,
                length_ms: 60000,
                chapters: None,
            },
            Chapter {
                title: "Chapter 2".to_string(),
                start_offset_ms: 60000,
                start_offset_sec: 60,
                length_ms: 60000,
                chapters: None,
            },
        ];

        let flattened = flatten_chapters(chapters.clone(), Some(": "));
        assert_eq!(flattened.len(), 2);
        assert_eq!(flattened[0].title, "Chapter 1");
    }

    #[test]
    fn test_combine_credits() {
        let mut chapters = vec![
            Chapter {
                title: "Opening Credits".to_string(),
                start_offset_ms: 0,
                start_offset_sec: 0,
                length_ms: 5000,
                chapters: None,
            },
            Chapter {
                title: "Chapter 1".to_string(),
                start_offset_ms: 5000,
                start_offset_sec: 5,
                length_ms: 60000,
                chapters: None,
            },
            Chapter {
                title: "End Credits".to_string(),
                start_offset_ms: 65000,
                start_offset_sec: 65,
                length_ms: 5000,
                chapters: None,
            },
        ];

        combine_credits(&mut chapters);
        assert_eq!(chapters.len(), 1);
        assert_eq!(chapters[0].title, "Chapter 1");
        assert_eq!(chapters[0].start_offset_ms, 0);
        assert_eq!(chapters[0].length_ms, 70000);
    }
}
