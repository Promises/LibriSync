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


//! Download manager for audiobook files
//!
//! # Reference C# Sources
//! - `FileLiberator/DownloadDecryptBook.cs` - Main download orchestration
//! - `FileLiberator/DownloadOptions.cs` - Download configuration
//! - `LibationFileManager/NamingTemplate/` - File naming system
//! - `ApplicationServices/ProcessBookQueue.cs` - Queue management patterns
//!
//! # Download Queue Management
//! - FIFO queue for downloads
//! - Configurable concurrent download limit (default: 3)
//! - Per-download state tracking
//! - Automatic retry on network failures
//! - Progress aggregation across all downloads
//!
//! # File Path Generation (from LibationFileManager)
//! - Template variables: {title}, {author}, {series}, {narrator}, {year}
//! - Sanitize filenames (remove invalid characters)
//! - Handle path length limits
//! - Avoid collisions (append number if exists)

use crate::error::{LibationError, Result};
use crate::api::client::AudibleClient;
use crate::download::progress::{DownloadProgress, DownloadState, ProgressCallback};
use crate::download::stream::{ResumableStream, download_to_file};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{RwLock, Semaphore, mpsc};
use serde::{Deserialize, Serialize};

/// Download configuration
///
/// Based on DownloadOptions.cs and user configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadConfig {
    /// Maximum concurrent downloads
    pub max_concurrent_downloads: usize,

    /// Output directory for downloaded files
    pub output_directory: PathBuf,

    /// Maximum retry attempts for failed downloads
    pub retry_attempts: u32,

    /// Auto-decrypt after download
    pub auto_decrypt: bool,

    /// File naming template (e.g., "{title} - {author}")
    pub file_naming_template: String,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            max_concurrent_downloads: 3,
            output_directory: PathBuf::from("."),
            retry_attempts: 3,
            auto_decrypt: true,
            file_naming_template: "{title} - {author}".to_string(),
        }
    }
}

/// Book metadata for download
#[derive(Debug, Clone)]
pub struct BookDownloadInfo {
    /// Audible ASIN
    pub asin: String,

    /// Book title
    pub title: String,

    /// Authors (comma-separated)
    pub authors: String,

    /// Series name (if part of a series)
    pub series: Option<String>,

    /// Narrators (comma-separated)
    pub narrators: Option<String>,

    /// Publication year
    pub year: Option<u32>,

    /// Download URL from license
    pub download_url: String,

    /// Total file size in bytes
    pub file_size: u64,

    /// Request headers for download
    pub request_headers: HashMap<String, String>,
}

/// Download task in the queue
#[derive(Debug, Clone)]
struct DownloadTask {
    /// Book information
    book: BookDownloadInfo,

    /// Output file path
    output_path: PathBuf,

    /// Number of retry attempts so far
    retry_count: u32,

    /// Current state
    state: DownloadState,

    /// Error message if failed
    error: Option<String>,
}

/// Download manager with queue and concurrency control
///
/// Port of download orchestration patterns from:
/// - FileLiberator/DownloadDecryptBook.cs (download flow)
/// - ApplicationServices/ProcessBookQueue.cs (queue management)
pub struct DownloadManager {
    /// Audible API client
    client: Arc<AudibleClient>,

    /// Configuration
    config: DownloadConfig,

    /// Active downloads (ASIN -> Task)
    active_downloads: Arc<RwLock<HashMap<String, DownloadTask>>>,

    /// Queued downloads
    download_queue: Arc<RwLock<Vec<DownloadTask>>>,

    /// Semaphore for concurrency control
    download_semaphore: Arc<Semaphore>,

    /// Progress callbacks (ASIN -> Callback)
    progress_callbacks: Arc<RwLock<HashMap<String, ProgressCallback>>>,
}

impl DownloadManager {
    /// Create new download manager
    pub fn new(client: AudibleClient, config: DownloadConfig) -> Self {
        let max_concurrent = config.max_concurrent_downloads;

        Self {
            client: Arc::new(client),
            config,
            active_downloads: Arc::new(RwLock::new(HashMap::new())),
            download_queue: Arc::new(RwLock::new(Vec::new())),
            download_semaphore: Arc::new(Semaphore::new(max_concurrent)),
            progress_callbacks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Enqueue a download
    ///
    /// Based on FileLiberator queue pattern
    pub async fn enqueue_download(
        &self,
        asin: String,
        book: BookDownloadInfo,
    ) -> Result<()> {
        // Generate output path
        let output_path = self.generate_output_path(&book)?;

        // Check if already exists
        if output_path.exists() && !self.should_overwrite(&output_path) {
            return Err(LibationError::InvalidState(
                format!("File already exists: {:?}", output_path)
            ));
        }

        // Create download task
        let task = DownloadTask {
            book,
            output_path,
            retry_count: 0,
            state: DownloadState::Queued,
            error: None,
        };

        // Add to queue
        let mut queue = self.download_queue.write().await;
        queue.push(task);

        Ok(())
    }

    /// Start a download from the queue
    ///
    /// This will block until a download slot is available
    pub async fn start_download(&self, asin: String) -> Result<()> {
        // Find task in queue
        let task = {
            let mut queue = self.download_queue.write().await;
            let pos = queue.iter().position(|t| t.book.asin == asin)
                .ok_or_else(|| LibationError::RecordNotFound(format!("ASIN not in queue: {}", asin)))?;
            queue.remove(pos)
        };

        // Spawn download task
        self.spawn_download_task(task).await;

        Ok(())
    }

    /// Start all queued downloads (respecting concurrency limit)
    pub async fn start_all_downloads(&self) -> Result<()> {
        loop {
            let task = {
                let mut queue = self.download_queue.write().await;
                if queue.is_empty() {
                    break;
                }
                queue.remove(0)
            };

            self.spawn_download_task(task).await;
        }

        Ok(())
    }

    /// Internal method to spawn a download task
    async fn spawn_download_task(&self, mut task: DownloadTask) {
        let asin = task.book.asin.clone();
        let client = Arc::clone(&self.client);
        let config = self.config.clone();
        let active = Arc::clone(&self.active_downloads);
        let semaphore = Arc::clone(&self.download_semaphore);
        let callbacks = Arc::clone(&self.progress_callbacks);

        tokio::spawn(async move {
            // Wait for download slot
            let _permit = semaphore.acquire().await.unwrap();

            // Mark as active
            task.state = DownloadState::Downloading;
            {
                let mut active_map = active.write().await;
                active_map.insert(asin.clone(), task.clone());
            }

            // Get progress callback
            let callback = {
                let cbs = callbacks.read().await;
                cbs.get(&asin).cloned()
            };

            // Perform download with retries
            let result = Self::download_with_retries(
                task.clone(),
                config.retry_attempts,
                callback.clone(),
            ).await;

            // Update state based on result
            match result {
                Ok(_) => {
                    task.state = DownloadState::Completed;
                    task.error = None;
                }
                Err(e) => {
                    task.state = DownloadState::Failed;
                    task.error = Some(e.to_string());
                }
            }

            // Remove from active
            {
                let mut active_map = active.write().await;
                active_map.remove(&asin);
            }

            // Notify completion via callback
            if let Some(cb) = callback {
                let mut progress = DownloadProgress::new(
                    task.book.asin.clone(),
                    task.book.title.clone(),
                    task.book.file_size,
                    task.book.file_size
                );
                progress.set_state(task.state);
                if let Some(error) = task.error.clone() {
                    progress.error_message = Some(error);
                }
                cb(progress);
            }
        });
    }

    /// Download with retry logic
    async fn download_with_retries(
        mut task: DownloadTask,
        max_retries: u32,
        callback: Option<ProgressCallback>,
    ) -> Result<()> {
        let mut last_error = None;

        while task.retry_count <= max_retries {
            match Self::perform_download(&task, callback.as_ref()).await {
                Ok(_) => return Ok(()),
                Err(e) => {
                    last_error = Some(e);
                    task.retry_count += 1;

                    if task.retry_count <= max_retries {
                        // Exponential backoff
                        let backoff = tokio::time::Duration::from_secs(2u64.pow(task.retry_count.min(5)));
                        tokio::time::sleep(backoff).await;
                    }
                }
            }
        }

        Err(last_error.unwrap())
    }

    /// Perform actual download
    async fn perform_download(
        task: &DownloadTask,
        callback: Option<&ProgressCallback>,
    ) -> Result<()> {
        let progress_fn = move |progress: DownloadProgress| {
            if let Some(cb) = callback {
                cb(progress);
            }
        };

        download_to_file(
            task.book.download_url.clone(),
            task.output_path.clone(),
            task.book.asin.clone(),
            task.book.title.clone(),
            task.book.request_headers.clone(),
            progress_fn,
        ).await
    }

    /// Pause a download
    pub async fn pause_download(&self, asin: &str) -> Result<()> {
        let mut active = self.active_downloads.write().await;

        if let Some(task) = active.get_mut(asin) {
            task.state = DownloadState::Paused;
            // Note: Actual pausing requires canceling the tokio task
            // This is a state marker for now
            Ok(())
        } else {
            Err(LibationError::RecordNotFound(format!("Download not active: {}", asin)))
        }
    }

    /// Resume a paused download
    pub async fn resume_download(&self, asin: &str) -> Result<()> {
        let task = {
            let mut active = self.active_downloads.write().await;
            active.get_mut(asin)
                .ok_or_else(|| LibationError::RecordNotFound(format!("Download not found: {}", asin)))?
                .clone()
        };

        // Re-spawn download task
        self.spawn_download_task(task).await;
        Ok(())
    }

    /// Cancel a download
    pub async fn cancel_download(&self, asin: &str) -> Result<()> {
        let mut active = self.active_downloads.write().await;

        if let Some(mut task) = active.remove(asin) {
            task.state = DownloadState::Cancelled;
            // TODO: Actually cancel the tokio task
            // This requires storing task handles
            Ok(())
        } else {
            Err(LibationError::RecordNotFound(format!("Download not active: {}", asin)))
        }
    }

    /// Retry a failed download
    pub async fn retry_failed_download(&self, asin: &str) -> Result<()> {
        // Check if in active with failed state
        let task = {
            let active = self.active_downloads.read().await;
            active.get(asin)
                .filter(|t| t.state == DownloadState::Failed)
                .cloned()
        };

        if let Some(mut task) = task {
            task.retry_count = 0;
            task.state = DownloadState::Queued;
            task.error = None;

            // Remove from active and re-queue
            {
                let mut active = self.active_downloads.write().await;
                active.remove(asin);
            }

            self.spawn_download_task(task).await;
            Ok(())
        } else {
            Err(LibationError::RecordNotFound(format!("No failed download for: {}", asin)))
        }
    }

    /// Get progress for a specific download
    pub async fn get_progress(&self, asin: &str) -> Option<DownloadProgress> {
        let active = self.active_downloads.read().await;
        active.get(asin).map(|task| {
            let mut progress = DownloadProgress::new(
                task.book.asin.clone(),
                task.book.title.clone(),
                0, // Would need to track this
                task.book.file_size
            );
            progress.set_state(task.state);
            if let Some(error) = &task.error {
                progress.error_message = Some(error.clone());
            }
            progress
        })
    }

    /// List all downloads (queued, active, completed)
    pub async fn list_downloads(&self) -> Vec<DownloadProgress> {
        let mut downloads = Vec::new();

        // Add queued
        {
            let queue = self.download_queue.read().await;
            for task in queue.iter() {
                let mut progress = DownloadProgress::new(
                    task.book.asin.clone(),
                    task.book.title.clone(),
                    0,
                    task.book.file_size
                );
                progress.set_state(task.state);
                if let Some(error) = &task.error {
                    progress.error_message = Some(error.clone());
                }
                downloads.push(progress);
            }
        }

        // Add active
        {
            let active = self.active_downloads.read().await;
            for task in active.values() {
                let mut progress = DownloadProgress::new(
                    task.book.asin.clone(),
                    task.book.title.clone(),
                    0,
                    task.book.file_size
                );
                progress.set_state(task.state);
                if let Some(error) = &task.error {
                    progress.error_message = Some(error.clone());
                }
                downloads.push(progress);
            }
        }

        downloads
    }

    /// Register a progress callback for a specific ASIN
    pub async fn register_progress_callback(&self, asin: String, callback: ProgressCallback) {
        let mut callbacks = self.progress_callbacks.write().await;
        callbacks.insert(asin, callback);
    }

    /// Generate output file path from book metadata and template
    ///
    /// Based on LibationFileManager/NamingTemplate system
    fn generate_output_path(&self, book: &BookDownloadInfo) -> Result<PathBuf> {
        let mut filename = self.config.file_naming_template.clone();

        // Replace template variables
        filename = filename.replace("{title}", &Self::sanitize_filename(&book.title));
        filename = filename.replace("{author}", &Self::sanitize_filename(&book.authors));

        if let Some(ref series) = book.series {
            filename = filename.replace("{series}", &Self::sanitize_filename(series));
        }

        if let Some(ref narrators) = book.narrators {
            filename = filename.replace("{narrator}", &Self::sanitize_filename(narrators));
        }

        if let Some(year) = book.year {
            filename = filename.replace("{year}", &year.to_string());
        }

        // Remove any remaining unreplaced variables
        filename = filename
            .replace("{series}", "")
            .replace("{narrator}", "")
            .replace("{year}", "");

        // Add extension
        filename.push_str(".aaxc");

        // Create full path
        let mut path = self.config.output_directory.clone();
        path.push(&filename);

        // Handle collisions by appending numbers
        let final_path = Self::avoid_collision(path);

        Ok(final_path)
    }

    /// Sanitize filename by removing invalid characters
    ///
    /// Based on FileUtility.SaferMoveToValidPath from Libation
    fn sanitize_filename(name: &str) -> String {
        let invalid_chars = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
        name.chars()
            .map(|c| if invalid_chars.contains(&c) { '_' } else { c })
            .collect::<String>()
            .trim()
            .to_string()
    }

    /// Avoid filename collisions by appending (1), (2), etc.
    ///
    /// Based on FileManager path collision handling
    fn avoid_collision(path: PathBuf) -> PathBuf {
        if !path.exists() {
            return path;
        }

        let parent = path.parent().unwrap();
        let stem = path.file_stem().unwrap().to_str().unwrap();
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        for i in 1..1000 {
            let new_filename = if extension.is_empty() {
                format!("{} ({})", stem, i)
            } else {
                format!("{} ({}).{}", stem, i, extension)
            };

            let new_path = parent.join(new_filename);
            if !new_path.exists() {
                return new_path;
            }
        }

        path // Give up after 1000 attempts
    }

    /// Check if we should overwrite an existing file
    fn should_overwrite(&self, _path: &Path) -> bool {
        // Could be configurable in the future
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(
            DownloadManager::sanitize_filename("Test: Book?"),
            "Test_ Book_"
        );

        assert_eq!(
            DownloadManager::sanitize_filename("Valid Name"),
            "Valid Name"
        );
    }

    // TODO: Fix this test - needs a valid Account object
    // #[test]
    // fn test_template_replacement() {
    //     let book = BookDownloadInfo {
    //         asin: "B001".to_string(),
    //         title: "Test Book".to_string(),
    //         authors: "John Doe".to_string(),
    //         series: Some("Test Series".to_string()),
    //         narrators: Some("Jane Smith".to_string()),
    //         year: Some(2023),
    //         download_url: "https://example.com".to_string(),
    //         file_size: 1000,
    //         request_headers: HashMap::new(),
    //     };

    //     let config = DownloadConfig {
    //         file_naming_template: "{title} - {author}".to_string(),
    //         ..Default::default()
    //     };

    //     // Need to create a proper Account object
    //     // let client = AudibleClient::new(account).unwrap();

    //     // let manager = DownloadManager::new(client, config);
    //     // let path = manager.generate_output_path(&book).unwrap();

    //     // assert!(path.to_str().unwrap().contains("Test Book - John Doe"));
    // }
}
