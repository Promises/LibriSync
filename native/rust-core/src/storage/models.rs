//! Database models for RN Audible
//!
//! This module contains all database entity models ported from Libation's
//! Entity Framework data layer to Rust with sqlx.
//!
//! # Reference C# Sources
//! - `DataLayer/EfClasses/Book.cs` - Book entity
//! - `DataLayer/EfClasses/LibraryBook.cs` - Library ownership
//! - `DataLayer/EfClasses/Contributor.cs` - Authors/narrators
//! - `DataLayer/EfClasses/Series.cs` - Series information
//! - `DataLayer/EfClasses/UserDefinedItem.cs` - User metadata
//! - `DataLayer/EfClasses/Rating.cs` - Rating information
//! - `DataLayer/AudioFormat.cs` - Audio format data
//!
//! # SQLite Adaptations
//! - Arrays stored as JSON strings (SQLite has no native array type)
//! - Enums stored as integers
//! - DateTime stored as TEXT in ISO 8601 format
//! - Owned entities (Rating, UserDefinedItem) embedded in parent table
//! - Many-to-many relationships use junction tables

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

// ============================================================================
// ENUMS
// ============================================================================

/// Content type for books
/// Maps to C# `ContentType` enum in Book.cs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i32)]
pub enum ContentType {
    Unknown = 0,
    Product = 1,
    Episode = 2,
    Parent = 4,
}

impl ContentType {
    pub fn from_i32(value: i32) -> Self {
        match value {
            1 => ContentType::Product,
            2 => ContentType::Episode,
            4 => ContentType::Parent,
            _ => ContentType::Unknown,
        }
    }
}

/// Liberation status for audiobooks and PDFs
/// Maps to C# `LiberatedStatus` enum in UserDefinedItem.cs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i32)]
pub enum LiberatedStatus {
    NotLiberated = 0,
    Liberated = 1,
    Error = 2,
    // Note: PartialDownload (0x1000) is application-state only, not persisted
}

impl LiberatedStatus {
    pub fn from_i32(value: i32) -> Self {
        match value {
            1 => LiberatedStatus::Liberated,
            2 => LiberatedStatus::Error,
            _ => LiberatedStatus::NotLiberated,
        }
    }
}

/// Contributor role (author, narrator, publisher)
/// Maps to C# `Role` enum in BookContributor.cs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i32)]
pub enum Role {
    Author = 1,
    Narrator = 2,
    Publisher = 3,
}

impl Role {
    pub fn from_i32(value: i32) -> Self {
        match value {
            1 => Role::Author,
            2 => Role::Narrator,
            3 => Role::Publisher,
            _ => Role::Author, // Default to author
        }
    }
}

/// Audio codec type
/// Maps to C# `Codec` enum in AudioFormat.cs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Codec {
    Unknown = 0,
    Mp3 = 1,
    AacLc = 2,
    XHeAac = 3,
    Ec3 = 4,
    Ac4 = 5,
}

impl Codec {
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => Codec::Mp3,
            2 => Codec::AacLc,
            3 => Codec::XHeAac,
            4 => Codec::Ec3,
            5 => Codec::Ac4,
            _ => Codec::Unknown,
        }
    }

    pub fn to_string(&self) -> &'static str {
        match self {
            Codec::Mp3 => "mp3",
            Codec::AacLc => "AAC-LC",
            Codec::XHeAac => "xHE-AAC",
            Codec::Ec3 => "EC-3",
            Codec::Ac4 => "AC-4",
            Codec::Unknown => "[Unknown]",
        }
    }
}

// ============================================================================
// VALUE OBJECTS
// ============================================================================

/// Rating information (overall, performance, story)
/// Maps to C# `Rating` class in Rating.cs (owned entity)
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Rating {
    pub overall_rating: f32,
    pub performance_rating: f32,
    pub story_rating: f32,
}

impl Rating {
    pub fn new(overall: f32, performance: f32, story: f32) -> Self {
        Self {
            overall_rating: overall,
            performance_rating: performance,
            story_rating: story,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.overall_rating == 0.0 && self.performance_rating == 0.0 && self.story_rating == 0.0
    }
}

impl Default for Rating {
    fn default() -> Self {
        Self::new(0.0, 0.0, 0.0)
    }
}

/// Audio format information
/// Maps to C# `AudioFormat` class in AudioFormat.cs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioFormat {
    pub codec: Codec,
    pub bit_rate: i32,      // kbps
    pub sample_rate: i32,   // Hz
    pub channel_count: i32,
}

impl AudioFormat {
    pub fn new(codec: Codec, bit_rate: i32, sample_rate: i32, channel_count: i32) -> Self {
        Self {
            codec,
            bit_rate,
            sample_rate,
            channel_count,
        }
    }

    pub fn is_default(&self) -> bool {
        matches!(self.codec, Codec::Unknown) && self.bit_rate == 0 && self.sample_rate == 0 && self.channel_count == 0
    }

    /// Serialize to i64 for database storage (matches C# serialization)
    /// Property     | Start | Num  |   Max   |
    /// -------------|-------|------|---------|
    /// Codec        |   35  |   4  |      15 |
    /// BitRate      |   23  |  12  |   4_095 |
    /// SampleRate   |    5  |  18  | 262_143 |
    /// ChannelCount |    0  |   5  |      31 |
    pub fn serialize(&self) -> i64 {
        ((self.codec as i64) << 35)
            | ((self.bit_rate as i64) << 23)
            | ((self.sample_rate as i64) << 5)
            | (self.channel_count as i64)
    }

    /// Deserialize from i64 (matches C# deserialization)
    pub fn deserialize(value: i64) -> Self {
        let codec = Codec::from_u8(((value >> 35) & 15) as u8);
        let bit_rate = ((value >> 23) & 4_095) as i32;
        let sample_rate = ((value >> 5) & 262_143) as i32;
        let channel_count = (value & 31) as i32;
        Self::new(codec, bit_rate, sample_rate, channel_count)
    }
}

impl Default for AudioFormat {
    fn default() -> Self {
        Self::new(Codec::Unknown, 0, 0, 0)
    }
}

// ============================================================================
// MAIN ENTITIES
// ============================================================================

/// Book entity - main audiobook metadata
/// Maps to C# `Book` class in Book.cs
///
/// **SQLite Adaptations:**
/// - `authors` and `narrators` stored as JSON arrays (many-to-many via junction)
/// - `rating` fields embedded (owned entity in C#)
/// - `content_type` stored as integer
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Book {
    /// Primary key (auto-increment)
    pub book_id: i64,

    // Immutable core fields
    /// Audible product ID (ASIN)
    pub audible_product_id: String,
    pub title: String,
    #[sqlx(default)]
    pub subtitle: Option<String>,
    pub description: String,
    pub length_in_minutes: i32,
    pub content_type: i32, // ContentType enum as integer
    pub locale: String,

    // Mutable metadata
    #[sqlx(default)]
    pub picture_id: Option<String>,
    #[sqlx(default)]
    pub picture_large: Option<String>,

    // Book details
    pub is_abridged: bool,
    pub is_spatial: bool,
    #[sqlx(default)]
    pub date_published: Option<NaiveDate>,
    #[sqlx(default)]
    pub language: Option<String>,

    // Product rating (aggregate community rating)
    pub rating_overall: f32,
    pub rating_performance: f32,
    pub rating_story: f32,

    // Additional metadata from API
    #[sqlx(default)]
    pub pdf_url: Option<String>,
    pub is_finished: bool,
    pub is_downloadable: bool,
    pub is_ayce: bool,
    #[sqlx(default)]
    pub origin_asin: Option<String>,
    #[sqlx(default)]
    pub episode_number: Option<i32>,
    #[sqlx(default)]
    pub content_delivery_type: Option<String>,

    // Timestamps
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Book {
    /// Get content type as enum
    pub fn get_content_type(&self) -> ContentType {
        ContentType::from_i32(self.content_type)
    }

    /// Get product rating
    pub fn get_rating(&self) -> Rating {
        Rating::new(self.rating_overall, self.rating_performance, self.rating_story)
    }

    /// Get title with subtitle (matches C# TitleWithSubtitle property)
    pub fn title_with_subtitle(&self) -> String {
        match &self.subtitle {
            Some(sub) if !sub.is_empty() => format!("{}: {}", self.title, sub),
            _ => self.title.clone(),
        }
    }
}

/// LibraryBook - represents user ownership of a book
/// Maps to C# `LibraryBook` class in LibraryBook.cs
///
/// In C#, this is a one-to-one relationship with Book (one book per user library).
/// The account field determines which Audible account owns the book.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct LibraryBook {
    pub book_id: i64, // Foreign key to Books table, also primary key
    pub date_added: DateTime<Utc>,
    pub account: String, // Account ID/email
    pub is_deleted: bool,
    pub absent_from_last_scan: bool,
}

/// UserDefinedItem - user-specific metadata for a book
/// Maps to C# `UserDefinedItem` class in UserDefinedItem.cs (owned entity)
///
/// **SQLite Adaptations:**
/// - Owned entity in C#, but stored as separate table with 1:1 relationship
/// - Tags stored as space-delimited lowercase string
/// - Rating fields embedded
/// - LiberatedStatus stored as integers
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct UserDefinedItem {
    pub book_id: i64, // Foreign key to Books, also primary key

    // User tags (space-delimited, lowercase, alphanumeric + underscore)
    #[sqlx(default)]
    pub tags: Option<String>,

    // User rating (personal, not aggregate)
    pub user_rating_overall: f32,
    pub user_rating_performance: f32,
    pub user_rating_story: f32,

    // Liberation status
    pub book_status: i32,        // LiberatedStatus enum
    #[sqlx(default)]
    pub pdf_status: Option<i32>, // LiberatedStatus enum, nullable

    // Download tracking
    #[sqlx(default)]
    pub last_downloaded: Option<DateTime<Utc>>,
    #[sqlx(default)]
    pub last_downloaded_version: Option<String>, // Libation version
    #[sqlx(default)]
    pub last_downloaded_format: Option<i64>,     // AudioFormat serialized
    #[sqlx(default)]
    pub last_downloaded_file_version: Option<String>, // Audio file version

    // User state
    pub is_finished: bool, // Has user finished listening?
}

impl UserDefinedItem {
    /// Get user rating
    pub fn get_user_rating(&self) -> Rating {
        Rating::new(
            self.user_rating_overall,
            self.user_rating_performance,
            self.user_rating_story,
        )
    }

    /// Get book status as enum
    pub fn get_book_status(&self) -> LiberatedStatus {
        LiberatedStatus::from_i32(self.book_status)
    }

    /// Get PDF status as enum (nullable)
    pub fn get_pdf_status(&self) -> Option<LiberatedStatus> {
        self.pdf_status.map(LiberatedStatus::from_i32)
    }

    /// Get audio format from serialized value
    pub fn get_audio_format(&self) -> Option<AudioFormat> {
        self.last_downloaded_format.map(AudioFormat::deserialize)
    }

    /// Parse tags into vector
    pub fn get_tags(&self) -> Vec<String> {
        self.tags
            .as_ref()
            .map(|t| {
                t.split_whitespace()
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Contributor - author, narrator, or publisher
/// Maps to C# `Contributor` class in Contributor.cs
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Contributor {
    pub contributor_id: i64,
    pub name: String,
    #[sqlx(default)]
    pub audible_contributor_id: Option<String>,
}

/// Series information
/// Maps to C# `Series` class in Series.cs
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Series {
    pub series_id: i64,
    pub audible_series_id: String,
    #[sqlx(default)]
    pub name: Option<String>,
}

/// Category/genre information
/// Maps to C# `Category` class in Category.cs
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Category {
    pub category_id: i64,
    #[sqlx(default)]
    pub audible_category_id: Option<String>,
    #[sqlx(default)]
    pub name: Option<String>,
}

/// Category ladder (hierarchical category path)
/// Maps to C# `CategoryLadder` class in CategoryLadder.cs
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct CategoryLadder {
    pub category_ladder_id: i64,
    pub audible_ladder_id: String,
    pub ladder: String, // JSON array of category IDs representing the path
}

/// Supplement (PDF) information
/// Maps to C# `Supplement` class in Supplement.cs (owned entity)
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Supplement {
    pub supplement_id: i64,
    pub book_id: i64,
    pub url: String,
}

// ============================================================================
// JUNCTION TABLES (Many-to-Many Relationships)
// ============================================================================

/// BookContributor - junction table for Book <-> Contributor
/// Maps to C# `BookContributor` class in BookContributor.cs
///
/// Composite primary key: (book_id, contributor_id, role)
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct BookContributor {
    pub book_id: i64,
    pub contributor_id: i64,
    pub role: i32, // Role enum (Author=1, Narrator=2, Publisher=3)
    pub order: i16, // Order within role (e.g., first author, second author)
}

impl BookContributor {
    pub fn get_role(&self) -> Role {
        Role::from_i32(self.role)
    }
}

/// SeriesBook - junction table for Series <-> Book
/// Maps to C# `SeriesBook` class in SeriesBook.cs
///
/// Composite primary key: (series_id, book_id)
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SeriesBook {
    pub series_id: i64,
    pub book_id: i64,
    #[sqlx(default)]
    pub order: Option<String>, // Order string (e.g., "1", "2.5", "Book 3")
    pub index: f32,             // Numeric index extracted from order string
}

/// BookCategory - junction table for Book <-> CategoryLadder
/// Maps to C# `BookCategory` class in BookCategory.cs
///
/// Composite primary key: (book_id, category_ladder_id)
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct BookCategory {
    pub book_id: i64,
    pub category_ladder_id: i64,
}

// ============================================================================
// NEW RECORD STRUCTS (for inserts)
// ============================================================================

/// New book record for insertion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewBook {
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
    pub date_published: Option<NaiveDate>,
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
}

impl NewBook {
    pub fn new(audible_product_id: String, title: String, locale: String) -> Self {
        Self {
            audible_product_id,
            title,
            subtitle: None,
            description: String::new(),
            length_in_minutes: 0,
            content_type: ContentType::Product as i32,
            locale,
            picture_id: None,
            picture_large: None,
            is_abridged: false,
            is_spatial: false,
            date_published: None,
            language: None,
            rating_overall: 0.0,
            rating_performance: 0.0,
            rating_story: 0.0,
            pdf_url: None,
            is_finished: false,
            is_downloadable: true,
            is_ayce: false,
            origin_asin: None,
            episode_number: None,
            content_delivery_type: None,
        }
    }
}

/// New library book record for insertion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewLibraryBook {
    pub book_id: i64,
    pub account: String,
}

/// New user defined item record for insertion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewUserDefinedItem {
    pub book_id: i64,
}

impl NewUserDefinedItem {
    pub fn new(book_id: i64) -> Self {
        Self { book_id }
    }
}

/// New contributor record for insertion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewContributor {
    pub name: String,
    pub audible_contributor_id: Option<String>,
}

impl NewContributor {
    pub fn new(name: String) -> Self {
        Self {
            name,
            audible_contributor_id: None,
        }
    }
}

/// New series record for insertion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewSeries {
    pub audible_series_id: String,
    pub name: Option<String>,
}

impl NewSeries {
    pub fn new(audible_series_id: String) -> Self {
        Self {
            audible_series_id,
            name: None,
        }
    }
}

/// New category record for insertion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewCategory {
    pub audible_category_id: Option<String>,
    pub name: Option<String>,
}

/// New category ladder record for insertion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewCategoryLadder {
    pub audible_ladder_id: String,
    pub ladder: String, // JSON array
}
