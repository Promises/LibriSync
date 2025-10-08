//! Database storage and models
//!
//! This module handles all database operations using SQLite.
//! It ports Libation's Entity Framework data layer to Rust with sqlx.
//!
//! # Reference C# Sources
//! - `DataLayer/EfClasses/` - Entity models (Book, LibraryBook, Series, etc.)
//! - `DataLayer/Configurations/` - EF Core configurations (table schema)
//! - `DataLayer/Migrations/` - Database migrations
//! - `DataLayer/LibationContext.cs` - DbContext (database access)
//!
//! # Database Schema
//! - Books: Core book metadata (title, ASIN, runtime, etc.)
//! - LibraryBooks: User ownership/library membership
//! - Contributors: Authors and narrators
//! - Series: Book series information
//! - Categories: Genres and tags
//! - Many-to-many junction tables for relationships
//!
//! # Usage Example
//! ```no_run
//! use rust_core::storage::{Database, queries, models::NewBook};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create database
//! let db = Database::new("./my_library.db").await?;
//!
//! // Insert a book
//! let new_book = NewBook::new(
//!     "B012345678".to_string(),
//!     "The Hobbit".to_string(),
//!     "us".to_string(),
//! );
//! let book_id = queries::insert_book(db.pool(), &new_book).await?;
//!
//! // Find book by ASIN
//! let book = queries::find_book_by_asin(db.pool(), "B012345678").await?;
//! # Ok(())
//! # }
//! ```

pub mod database;
pub mod migrations;
pub mod models;
pub mod queries;

// Re-export commonly used types
pub use database::{Database, DatabaseStats};
pub use models::{
    AudioFormat, Book, BookCategory, BookContributor, Category, CategoryLadder, Codec,
    ContentType, Contributor, LiberatedStatus, LibraryBook, NewBook, NewCategory,
    NewCategoryLadder, NewContributor, NewLibraryBook, NewSeries, NewUserDefinedItem, Rating,
    Role, Series, SeriesBook, Supplement, UserDefinedItem,
};
