//! Database connection and management
//!
//! This module handles database connection pooling, initialization, and maintenance.
//! Ported from Libation's LibationContext.cs.
//!
//! # Reference C# Sources
//! - `DataLayer/LibationContext.cs` - Main DbContext class
//! - `DataLayer/LibationContextFactory.cs` - Context factory
//!
//! # Database Location
//! - Desktop (macOS): ~/Library/Application Support/RNAudible/database.db
//! - Desktop (Linux): ~/.local/share/RNAudible/database.db
//! - Desktop (Windows): %APPDATA%/RNAudible/database.db
//! - Android: app-specific data directory (context.getDatabasePath())
//! - iOS: app-specific documents directory
//!
//! # SQLite Configuration
//! - WAL mode for better concurrency
//! - Foreign keys enabled
//! - Incremental auto-vacuum for space efficiency
//! - Normal synchronous mode (balance safety/speed)

use crate::error::{LibationError, Result};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions},
    ConnectOptions, Executor,
};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

/// Database manager - handles connection pooling and operations
/// Maps to C# `LibationContext` class in LibationContext.cs
#[derive(Debug, Clone)]
pub struct Database {
    pool: SqlitePool,
    path: Option<PathBuf>, // None for in-memory databases
}

impl Database {
    /// Create new database connection with migrations
    ///
    /// # Arguments
    /// * `database_path` - Path to SQLite database file (will be created if doesn't exist)
    ///
    /// # Errors
    /// Returns error if:
    /// - Parent directory doesn't exist and can't be created
    /// - Database file can't be opened
    /// - Migrations fail
    /// - Pragma configuration fails
    pub async fn new<P: AsRef<Path>>(database_path: P) -> Result<Self> {
        let path = database_path.as_ref();

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    LibationError::FileIoError(format!(
                        "Failed to create database directory {}: {}",
                        parent.display(),
                        e
                    ))
                })?;
            }
        }

        // Create connection options
        let connection_string = format!("sqlite://{}?mode=rwc", path.display());
        let mut connect_opts = SqliteConnectOptions::from_str(&connection_string)?
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(30));

        // Disable logging for production use
        connect_opts = connect_opts.disable_statement_logging();

        // Create connection pool
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(30))
            .connect_with(connect_opts)
            .await?;

        // Configure database with pragmas
        Self::configure_database(&pool).await?;

        // Run migrations
        let db = Self {
            pool,
            path: Some(path.to_path_buf()),
        };
        db.migrate().await?;

        Ok(db)
    }

    /// Create in-memory database for testing
    ///
    /// # Errors
    /// Returns error if database creation or migration fails
    pub async fn new_in_memory() -> Result<Self> {
        let connect_opts = SqliteConnectOptions::from_str("sqlite::memory:")?
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
            .disable_statement_logging();

        let pool = SqlitePoolOptions::new()
            .max_connections(1) // In-memory DB typically single-threaded
            .connect_with(connect_opts)
            .await?;

        Self::configure_database(&pool).await?;

        let db = Self { pool, path: None };
        db.migrate().await?;

        Ok(db)
    }

    /// Configure database with pragmas
    ///
    /// Sets up SQLite pragmas for optimal performance and reliability:
    /// - WAL journal mode (already set in connect options)
    /// - Foreign keys enabled (already set in connect options)
    /// - Incremental auto-vacuum
    async fn configure_database(pool: &SqlitePool) -> Result<()> {
        // Enable incremental auto-vacuum
        sqlx::query("PRAGMA auto_vacuum = INCREMENTAL")
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Run database migrations
    ///
    /// Applies all pending migrations to bring the database schema up to date.
    /// Migrations are run automatically when creating a new database connection.
    pub async fn migrate(&self) -> Result<()> {
        // Run migrations defined in migrations.rs
        crate::storage::migrations::run_migrations(&self.pool)
            .await
            .map_err(|e| LibationError::MigrationFailed(e.to_string()))?;

        Ok(())
    }

    /// Get reference to the connection pool
    ///
    /// Use this to execute queries directly on the pool
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Get database file path
    ///
    /// Returns `None` for in-memory databases
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Close database and release all connections
    ///
    /// This will wait for all active connections to finish before closing.
    pub async fn close(self) -> Result<()> {
        self.pool.close().await;
        Ok(())
    }

    /// Get default database path for the platform
    ///
    /// Returns platform-specific application data directory path:
    /// - macOS: ~/Library/Application Support/RNAudible/database.db
    /// - Linux: ~/.local/share/RNAudible/database.db
    /// - Windows: %APPDATA%/RNAudible/database.db
    ///
    /// Note: For Android/iOS, use platform-specific APIs to get app data directory
    pub fn get_default_path() -> PathBuf {
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("RNAudible")
                .join("database.db")
        }

        #[cfg(target_os = "linux")]
        {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("RNAudible")
                .join("database.db")
        }

        #[cfg(target_os = "windows")]
        {
            let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(appdata)
                .join("RNAudible")
                .join("database.db")
        }

        #[cfg(target_os = "android")]
        {
            // For Android, this should be overridden by the app
            // Use context.getDatabasePath("database.db") from Java/Kotlin
            PathBuf::from("/data/data/com.rnaudible/databases/database.db")
        }

        #[cfg(target_os = "ios")]
        {
            // For iOS, this should be overridden by the app
            // Use FileManager.default.urls(for: .documentDirectory) from Swift
            PathBuf::from("./database.db")
        }

        #[cfg(not(any(
            target_os = "macos",
            target_os = "linux",
            target_os = "windows",
            target_os = "android",
            target_os = "ios"
        )))]
        {
            PathBuf::from("./database.db")
        }
    }

    /// Vacuum database to reclaim unused space
    ///
    /// This should be run periodically (e.g., weekly) to optimize database size.
    /// The operation may take some time for large databases.
    pub async fn vacuum(&self) -> Result<()> {
        sqlx::query("VACUUM").execute(&self.pool).await?;
        Ok(())
    }

    /// Run incremental vacuum to reclaim some space
    ///
    /// This is faster than full VACUUM and can be run more frequently.
    /// Pages parameter specifies how many pages to free (0 = all available).
    pub async fn incremental_vacuum(&self, pages: i32) -> Result<()> {
        let query = if pages > 0 {
            format!("PRAGMA incremental_vacuum({})", pages)
        } else {
            "PRAGMA incremental_vacuum".to_string()
        };
        sqlx::query(&query).execute(&self.pool).await?;
        Ok(())
    }

    /// Get database size in bytes
    ///
    /// Returns size of the main database file.
    /// For in-memory databases, returns 0.
    pub async fn get_size(&self) -> Result<u64> {
        if let Some(path) = &self.path {
            let metadata = std::fs::metadata(path).map_err(|e| {
                LibationError::FileIoError(format!(
                    "Failed to get database size for {}: {}",
                    path.display(),
                    e
                ))
            })?;
            Ok(metadata.len())
        } else {
            // In-memory database
            Ok(0)
        }
    }

    /// Get database statistics
    ///
    /// Returns useful database information like page count, page size, etc.
    pub async fn get_stats(&self) -> Result<DatabaseStats> {
        let page_count: i64 = sqlx::query_scalar("PRAGMA page_count")
            .fetch_one(&self.pool)
            .await?;

        let page_size: i64 = sqlx::query_scalar("PRAGMA page_size")
            .fetch_one(&self.pool)
            .await?;

        let freelist_count: i64 = sqlx::query_scalar("PRAGMA freelist_count")
            .fetch_one(&self.pool)
            .await?;

        Ok(DatabaseStats {
            page_count: page_count as u64,
            page_size: page_size as u64,
            freelist_count: freelist_count as u64,
            total_size: (page_count * page_size) as u64,
            unused_size: (freelist_count * page_size) as u64,
        })
    }

    /// Checkpoint WAL file to main database
    ///
    /// This writes all WAL changes back to the main database file.
    /// Useful before backup or export operations.
    pub async fn checkpoint(&self) -> Result<()> {
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Export database to file
    ///
    /// Creates a copy of the database at the specified path.
    /// Automatically checkpoints WAL before export.
    ///
    /// # Arguments
    /// * `output_path` - Destination path for the database copy
    ///
    /// # Errors
    /// Returns error if:
    /// - Source database path is unknown (in-memory database)
    /// - Checkpoint fails
    /// - File copy fails
    pub async fn export<P: AsRef<Path>>(&self, output_path: P) -> Result<()> {
        let source_path = self
            .path
            .as_ref()
            .ok_or_else(|| LibationError::InvalidState("Cannot export in-memory database".to_string()))?;

        // Checkpoint WAL to ensure all data is in main file
        self.checkpoint().await?;

        // Copy database file
        std::fs::copy(source_path, output_path.as_ref()).map_err(|e| {
            LibationError::FileIoError(format!(
                "Failed to export database to {}: {}",
                output_path.as_ref().display(),
                e
            ))
        })?;

        Ok(())
    }

    /// Optimize database
    ///
    /// Runs ANALYZE to update query planner statistics.
    /// Should be run after bulk operations or periodically (e.g., weekly).
    pub async fn optimize(&self) -> Result<()> {
        sqlx::query("ANALYZE").execute(&self.pool).await?;
        Ok(())
    }

    /// Check database integrity
    ///
    /// Runs SQLite integrity check and returns true if database is okay.
    /// This is a thorough check that scans the entire database.
    pub async fn check_integrity(&self) -> Result<bool> {
        let result: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&self.pool)
            .await?;

        Ok(result == "ok")
    }

    /// Quick integrity check
    ///
    /// Faster version of integrity_check that only checks key structures.
    pub async fn quick_check(&self) -> Result<bool> {
        let result: String = sqlx::query_scalar("PRAGMA quick_check")
            .fetch_one(&self.pool)
            .await?;

        Ok(result == "ok")
    }
}

/// Database statistics
#[derive(Debug, Clone)]
pub struct DatabaseStats {
    /// Total number of pages in database
    pub page_count: u64,
    /// Size of each page in bytes
    pub page_size: u64,
    /// Number of free pages (unused space)
    pub freelist_count: u64,
    /// Total size of database (page_count * page_size)
    pub total_size: u64,
    /// Unused space (freelist_count * page_size)
    pub unused_size: u64,
}

impl DatabaseStats {
    /// Get percentage of unused space
    pub fn unused_percentage(&self) -> f64 {
        if self.total_size == 0 {
            0.0
        } else {
            (self.unused_size as f64 / self.total_size as f64) * 100.0
        }
    }

    /// Check if vacuum is recommended (>20% unused space)
    pub fn should_vacuum(&self) -> bool {
        self.unused_percentage() > 20.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_in_memory_database() {
        let db = Database::new_in_memory().await.expect("Failed to create in-memory database");

        // Verify database is accessible
        let result: i64 = sqlx::query_scalar("SELECT 1")
            .fetch_one(db.pool())
            .await
            .expect("Failed to query database");

        assert_eq!(result, 1);
    }

    #[tokio::test]
    async fn test_database_stats() {
        let db = Database::new_in_memory().await.expect("Failed to create database");
        let stats = db.get_stats().await.expect("Failed to get stats");

        assert!(stats.page_size > 0);
        assert!(stats.page_count > 0);
    }

    #[tokio::test]
    async fn test_integrity_check() {
        let db = Database::new_in_memory().await.expect("Failed to create database");
        let is_ok = db.check_integrity().await.expect("Failed to check integrity");

        assert!(is_ok, "Database integrity check failed");
    }
}
