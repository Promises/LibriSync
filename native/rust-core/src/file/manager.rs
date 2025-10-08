//! File operations and management
//!
//! # Reference C# Sources
//! - `FileManager/FileUtility.cs` - File operations (move, copy, delete)
//! - `LibationFileManager/AudibleFileStorage.cs` - Audiobook file management
//!
//! # Key Operations
//! - Safe file moves (atomic when possible, with retry)
//! - File existence checks
//! - Directory creation
//! - Disk space checks
//! - File cleanup (temp files, old versions)

use crate::audio::metadata::AudioMetadata;
use crate::error::{LibationError, Result};
use crate::file::paths::{avoid_collision, get_safe_filename, PathTemplate};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::fs;
use tokio::time::sleep;

/// Maximum retry attempts for file operations
const MAX_RETRY_ATTEMPTS: u32 = 3;

/// Delay between retry attempts
const RETRY_DELAY: Duration = Duration::from_millis(100);

/// File manager for safe file operations
///
/// # Reference: `FileManager/FileUtility.cs`
#[derive(Debug)]
pub struct FileManager {
    /// Base library directory
    library_path: PathBuf,
}

impl FileManager {
    /// Create a new file manager
    pub fn new(library_path: PathBuf) -> Self {
        Self { library_path }
    }

    /// Get the library path
    pub fn library_path(&self) -> &Path {
        &self.library_path
    }

    /// Safe move operation with retry
    ///
    /// # Reference: `FileManager/FileUtility.cs` SaferMove()
    ///
    /// # Algorithm (from Libation)
    /// 1. Check source exists
    /// 2. Delete destination if it exists
    /// 3. Create destination directory
    /// 4. Move file (atomic on same filesystem)
    /// 5. Retry up to 3 times on failure
    pub async fn safe_move(&self, source: &Path, destination: &Path) -> Result<()> {
        let mut attempts = 0;

        loop {
            attempts += 1;

            match self.try_move(source, destination).await {
                Ok(()) => return Ok(()),
                Err(e) if attempts >= MAX_RETRY_ATTEMPTS => {
                    return Err(LibationError::FileIoError(format!(
                        "Failed to move file after {} attempts: {} -> {}: {}",
                        MAX_RETRY_ATTEMPTS,
                        source.display(),
                        destination.display(),
                        e
                    )));
                }
                Err(_) => {
                    sleep(RETRY_DELAY).await;
                    continue;
                }
            }
        }
    }

    /// Try to move file once
    async fn try_move(&self, source: &Path, destination: &Path) -> Result<()> {
        // Check source exists
        if !Self::file_exists(source).await {
            // Not an error if source doesn't exist (already moved?)
            return Ok(());
        }

        // Delete destination if it exists
        if Self::file_exists(destination).await {
            Self::safe_delete_once(destination).await?;
        }

        // Create parent directory
        if let Some(parent) = destination.parent() {
            self.ensure_directory_exists(parent).await?;
        }

        // Try atomic move (rename)
        fs::rename(source, destination).await.map_err(|e| {
            LibationError::FileIoError(format!(
                "Move failed: {} -> {}: {}",
                source.display(),
                destination.display(),
                e
            ))
        })?;

        Ok(())
    }

    /// Safe copy operation with retry
    ///
    /// # Reference: `FileManager/FileUtility.cs` SaferCopy() (inferred)
    pub async fn safe_copy(&self, source: &Path, destination: &Path) -> Result<()> {
        let mut attempts = 0;

        loop {
            attempts += 1;

            match self.try_copy(source, destination).await {
                Ok(()) => return Ok(()),
                Err(e) if attempts >= MAX_RETRY_ATTEMPTS => {
                    return Err(LibationError::FileIoError(format!(
                        "Failed to copy file after {} attempts: {} -> {}: {}",
                        MAX_RETRY_ATTEMPTS,
                        source.display(),
                        destination.display(),
                        e
                    )));
                }
                Err(_) => {
                    sleep(RETRY_DELAY).await;
                    continue;
                }
            }
        }
    }

    /// Try to copy file once
    async fn try_copy(&self, source: &Path, destination: &Path) -> Result<()> {
        // Check source exists
        if !Self::file_exists(source).await {
            return Err(LibationError::FileNotFound(source.display().to_string()));
        }

        // Create parent directory
        if let Some(parent) = destination.parent() {
            self.ensure_directory_exists(parent).await?;
        }

        // Copy file
        fs::copy(source, destination).await.map_err(|e| {
            LibationError::FileIoError(format!(
                "Copy failed: {} -> {}: {}",
                source.display(),
                destination.display(),
                e
            ))
        })?;

        Ok(())
    }

    /// Safe delete operation with retry
    ///
    /// # Reference: `FileManager/FileUtility.cs` SaferDelete()
    pub async fn safe_delete(&self, path: &Path) -> Result<()> {
        let mut attempts = 0;

        loop {
            attempts += 1;

            match Self::safe_delete_once(path).await {
                Ok(()) => return Ok(()),
                Err(e) if attempts >= MAX_RETRY_ATTEMPTS => {
                    return Err(LibationError::FileIoError(format!(
                        "Failed to delete file after {} attempts: {}: {}",
                        MAX_RETRY_ATTEMPTS,
                        path.display(),
                        e
                    )));
                }
                Err(_) => {
                    sleep(RETRY_DELAY).await;
                    continue;
                }
            }
        }
    }

    /// Try to delete file once
    async fn safe_delete_once(path: &Path) -> Result<()> {
        // Not an error if file doesn't exist
        if !Self::file_exists(path).await {
            return Ok(());
        }

        fs::remove_file(path).await.map_err(|e| {
            LibationError::FileIoError(format!("Delete failed: {}: {}", path.display(), e))
        })?;

        Ok(())
    }

    /// Ensure directory exists, creating parent directories as needed
    ///
    /// # Reference: `FileManager/FileUtility.cs` (uses Directory.CreateDirectory)
    pub async fn ensure_directory_exists(&self, path: &Path) -> Result<()> {
        if path.exists() {
            return Ok(());
        }

        fs::create_dir_all(path).await.map_err(|e| {
            LibationError::FileIoError(format!(
                "Failed to create directory {}: {}",
                path.display(),
                e
            ))
        })
    }

    /// Check if file exists
    pub async fn file_exists(path: &Path) -> bool {
        fs::try_exists(path).await.unwrap_or(false)
    }

    /// Get file size in bytes
    pub async fn get_file_size(path: &Path) -> Result<u64> {
        let metadata = fs::metadata(path).await.map_err(|e| {
            LibationError::FileIoError(format!("Failed to get file size {}: {}", path.display(), e))
        })?;

        Ok(metadata.len())
    }

    /// Check available disk space
    ///
    /// # Reference: `FileManager/FileUtility.cs` (inferred from Libation behavior)
    pub async fn check_disk_space(&self, path: &Path, required_bytes: u64) -> Result<bool> {
        // Use fs2 crate for cross-platform disk space checking
        // For now, return true (optimistic)
        // TODO: Implement actual disk space checking using fs2 or sysinfo crate
        let _ = (path, required_bytes);
        Ok(true)
    }

    /// Verify file integrity by checking size
    pub async fn verify_file_integrity(&self, path: &Path, expected_size: u64) -> Result<bool> {
        if !Self::file_exists(path).await {
            return Ok(false);
        }

        let actual_size = Self::get_file_size(path).await?;
        Ok(actual_size == expected_size)
    }

    /// Organize audiobook file: move to library with proper naming
    ///
    /// # Reference: Combined from `FileManager/FileUtility.cs` and `LibationFileManager/`
    ///
    /// # Algorithm
    /// 1. Generate target path from template
    /// 2. Ensure parent directories exist
    /// 3. Check disk space (if size provided)
    /// 4. Move file to target location (avoiding collisions)
    /// 5. Return new path
    pub async fn organize_audiobook(
        &self,
        source_path: &Path,
        metadata: &AudioMetadata,
        template: &PathTemplate,
        extension: &str,
    ) -> Result<PathBuf> {
        // Check source exists
        if !Self::file_exists(source_path).await {
            return Err(LibationError::FileNotFound(source_path.display().to_string()));
        }

        // Get source file size for verification
        let source_size = Self::get_file_size(source_path).await?;

        // Check disk space
        self.check_disk_space(&self.library_path, source_size)
            .await?;

        // Generate safe filename (handles collisions)
        let target_path = get_safe_filename(&self.library_path, metadata, template, extension)?;

        // Ensure parent directory exists
        if let Some(parent) = target_path.parent() {
            self.ensure_directory_exists(parent).await?;
        }

        // Move file
        self.safe_move(source_path, &target_path).await?;

        // Verify move succeeded
        if !self
            .verify_file_integrity(&target_path, source_size)
            .await?
        {
            return Err(LibationError::FileIoError(format!(
                "File verification failed after move: {}",
                target_path.display()
            )));
        }

        Ok(target_path)
    }

    /// Cleanup empty parent directories after file deletion
    ///
    /// # Reference: Common Libation pattern (inferred)
    pub async fn cleanup_empty_directories(&self, path: &Path) -> Result<()> {
        let mut current = path.parent();

        while let Some(dir) = current {
            // Stop if we reach the library root
            if dir == self.library_path {
                break;
            }

            // Check if directory is empty
            let mut entries = fs::read_dir(dir).await.map_err(|e| {
                LibationError::FileIoError(format!(
                    "Failed to read directory {}: {}",
                    dir.display(),
                    e
                ))
            })?;

            // If directory has any entries, stop
            if entries.next_entry().await.map_err(|e| {
                LibationError::FileIoError(format!(
                    "Failed to check directory entries {}: {}",
                    dir.display(),
                    e
                ))
            })?.is_some() {
                break;
            }

            // Directory is empty, remove it
            fs::remove_dir(dir).await.map_err(|e| {
                LibationError::FileIoError(format!(
                    "Failed to remove empty directory {}: {}",
                    dir.display(),
                    e
                ))
            })?;

            // Move to parent
            current = dir.parent();
        }

        Ok(())
    }

    /// Atomic write: write to temp file, then rename
    ///
    /// # Reference: Common pattern for safe file writing
    pub async fn atomic_write(&self, path: &Path, contents: &[u8]) -> Result<()> {
        // Create temp file in same directory
        let temp_path = if let Some(parent) = path.parent() {
            parent.join(format!(
                ".{}.tmp",
                path.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("file")
            ))
        } else {
            PathBuf::from(format!(
                ".{}.tmp",
                path.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("file")
            ))
        };

        // Write to temp file
        fs::write(&temp_path, contents).await.map_err(|e| {
            LibationError::FileIoError(format!(
                "Failed to write temp file {}: {}",
                temp_path.display(),
                e
            ))
        })?;

        // Sync to disk
        let file = fs::OpenOptions::new()
            .write(true)
            .open(&temp_path)
            .await
            .map_err(|e| {
                LibationError::FileIoError(format!(
                    "Failed to open temp file for sync {}: {}",
                    temp_path.display(),
                    e
                ))
            })?;

        file.sync_all().await.map_err(|e| {
            LibationError::FileIoError(format!(
                "Failed to sync temp file {}: {}",
                temp_path.display(),
                e
            ))
        })?;

        drop(file);

        // Atomic rename
        fs::rename(&temp_path, path).await.map_err(|e| {
            LibationError::FileIoError(format!(
                "Failed to rename temp file {} to {}: {}",
                temp_path.display(),
                path.display(),
                e
            ))
        })?;

        Ok(())
    }

    /// Get temp directory
    pub fn get_temp_directory() -> PathBuf {
        std::env::temp_dir()
    }

    /// Cleanup temp files matching pattern
    pub async fn cleanup_temp_files(&self, pattern: &str) -> Result<usize> {
        let temp_dir = Self::get_temp_directory();
        let mut count = 0;

        let mut entries = fs::read_dir(&temp_dir).await.map_err(|e| {
            LibationError::FileIoError(format!(
                "Failed to read temp directory {}: {}",
                temp_dir.display(),
                e
            ))
        })?;

        while let Some(entry) = entries.next_entry().await.map_err(|e| {
            LibationError::FileIoError(format!(
                "Failed to read directory entry in {}: {}",
                temp_dir.display(),
                e
            ))
        })? {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                // Simple pattern matching (contains)
                if name.contains(pattern) {
                    if let Ok(()) = Self::safe_delete_once(&path).await {
                        count += 1;
                    }
                }
            }
        }

        Ok(count)
    }

    /// Validate library structure (check for issues)
    pub async fn validate_library_structure(&self) -> Result<Vec<String>> {
        let mut issues = Vec::new();

        // Check if library directory exists
        if !self.library_path.exists() {
            issues.push(format!(
                "Library directory does not exist: {}",
                self.library_path.display()
            ));
            return Ok(issues);
        }

        // Check if library directory is readable
        if fs::read_dir(&self.library_path).await.is_err() {
            issues.push(format!(
                "Library directory is not readable: {}",
                self.library_path.display()
            ));
        }

        // TODO: Add more validation checks:
        // - Check for duplicate files
        // - Check for orphaned files
        // - Check for invalid filenames
        // - Check for broken symbolic links

        Ok(issues)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::metadata::SeriesInfo;
    use tempfile::TempDir;

    fn test_metadata() -> AudioMetadata {
        AudioMetadata {
            title: "Test Book".to_string(),
            authors: vec!["John Doe".to_string()],
            narrators: vec!["Jane Smith".to_string()],
            publisher: Some("Test Publisher".to_string()),
            publication_date: Some("2023".to_string()),
            language: Some("en".to_string()),
            series: Some(SeriesInfo {
                name: "Test Series".to_string(),
                position: Some("1".to_string()),
            }),
            description: None,
            genres: vec!["Fiction".to_string()],
            runtime_minutes: Some(600),
            asin: Some("B001TEST".to_string()),
            cover_art_url: None,
        }
    }

    #[tokio::test]
    async fn test_ensure_directory_exists() {
        let temp_dir = TempDir::new().unwrap();
        let library_path = temp_dir.path().join("library");

        let manager = FileManager::new(library_path.clone());
        manager
            .ensure_directory_exists(&library_path)
            .await
            .unwrap();

        assert!(library_path.exists());
    }

    #[tokio::test]
    async fn test_safe_move() {
        let temp_dir = TempDir::new().unwrap();
        let library_path = temp_dir.path().join("library");
        let manager = FileManager::new(library_path.clone());

        // Create source file
        let source = temp_dir.path().join("source.txt");
        fs::write(&source, b"test content").await.unwrap();

        // Move to destination
        let dest = library_path.join("dest.txt");
        manager.safe_move(&source, &dest).await.unwrap();

        assert!(!source.exists());
        assert!(dest.exists());
        let content = fs::read_to_string(&dest).await.unwrap();
        assert_eq!(content, "test content");
    }

    #[tokio::test]
    async fn test_safe_copy() {
        let temp_dir = TempDir::new().unwrap();
        let library_path = temp_dir.path().join("library");
        let manager = FileManager::new(library_path.clone());

        // Create source file
        let source = temp_dir.path().join("source.txt");
        fs::write(&source, b"test content").await.unwrap();

        // Copy to destination
        let dest = library_path.join("dest.txt");
        manager.safe_copy(&source, &dest).await.unwrap();

        assert!(source.exists());
        assert!(dest.exists());
        let content = fs::read_to_string(&dest).await.unwrap();
        assert_eq!(content, "test content");
    }

    #[tokio::test]
    async fn test_safe_delete() {
        let temp_dir = TempDir::new().unwrap();
        let library_path = temp_dir.path().join("library");
        let manager = FileManager::new(library_path);

        // Create file
        let file = temp_dir.path().join("file.txt");
        fs::write(&file, b"test content").await.unwrap();

        // Delete
        manager.safe_delete(&file).await.unwrap();
        assert!(!file.exists());

        // Delete non-existent file should not error
        manager.safe_delete(&file).await.unwrap();
    }

    #[tokio::test]
    async fn test_atomic_write() {
        let temp_dir = TempDir::new().unwrap();
        let library_path = temp_dir.path().join("library");
        let manager = FileManager::new(library_path.clone());
        manager.ensure_directory_exists(&library_path).await.unwrap();

        let file = library_path.join("atomic.txt");
        manager
            .atomic_write(&file, b"atomic content")
            .await
            .unwrap();

        assert!(file.exists());
        let content = fs::read_to_string(&file).await.unwrap();
        assert_eq!(content, "atomic content");
    }

    #[tokio::test]
    async fn test_organize_audiobook() {
        let temp_dir = TempDir::new().unwrap();
        let library_path = temp_dir.path().join("library");
        let manager = FileManager::new(library_path.clone());

        // Create source file
        let source = temp_dir.path().join("source.m4b");
        fs::write(&source, b"audio content").await.unwrap();

        // Organize
        let template = PathTemplate::default_audiobook();
        let metadata = test_metadata();

        let result = manager
            .organize_audiobook(&source, &metadata, &template, "m4b")
            .await
            .unwrap();

        assert!(!source.exists());
        assert!(result.exists());
        assert!(result.to_string_lossy().contains("John Doe"));
        assert!(result.to_string_lossy().contains("Test Book"));
    }
}
