//! HTTP streaming with resume support
//!
//! # Reference C# Sources
//! - `AaxDecrypter/NetworkFileStream.cs` - Resumable HTTP streaming (lines 13-421)
//! - `AaxDecrypter/NetworkFileStreamPersister.cs` - Persistent state management
//!
//! # Key Features from NetworkFileStream.cs
//! - Resumable downloads using HTTP Range headers
//! - Progress tracking with automatic state persistence
//! - Retry logic for connection drops (lines 182-210)
//! - Buffered writing with periodic flushes (line 69: DATA_FLUSH_SZ = 1MB)
//! - Download speed throttling support (lines 46-48, 282-296)
//! - Chunk size: 8KB (line 65: DOWNLOAD_BUFF_SZ = 8 * 1024)
//!
//! # Resume Mechanism (from NetworkFileStream.cs lines 220-244)
//! 1. Send Range header: bytes={WritePosition}-
//! 2. Server responds with 206 Partial Content
//! 3. Verify ContentRange.Length matches expected total size
//! 4. Continue writing from WritePosition

use crate::error::{LibationError, Result};
use crate::download::progress::{DownloadProgress, ProgressTracker, DownloadState as ProgressState};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncWriteExt, BufWriter};
use reqwest::{Client, StatusCode};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

// Constants from NetworkFileStream.cs
const DOWNLOAD_BUFF_SZ: usize = 8 * 1024; // 8KB chunks (line 65)
const DATA_FLUSH_SZ: u64 = 1024 * 1024; // Flush every 1MB (line 69)
const MAX_RETRIES: u32 = 5; // Maximum retry attempts

/// Persistent download state for resume support
///
/// Based on NetworkFileStream.cs JSON serialization (lines 21-38)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamState {
    /// HTTP(s) URL of the file
    pub url: String,

    /// Local file path to save to
    pub save_file_path: PathBuf,

    /// Total content length in bytes
    pub content_length: u64,

    /// Bytes written and flushed to disk
    pub write_position: u64,

    /// Timestamp of last save
    pub timestamp: String,

    /// Request headers to include
    #[serde(default)]
    pub request_headers: std::collections::HashMap<String, String>,
}

impl StreamState {
    /// Create new stream state
    pub fn new(url: String, save_file_path: PathBuf) -> Self {
        Self {
            url,
            save_file_path,
            content_length: 0,
            write_position: 0,
            timestamp: chrono::Utc::now().to_rfc3339(),
            request_headers: std::collections::HashMap::new(),
        }
    }

    /// Get the state file path (.json)
    pub fn state_file_path(&self) -> PathBuf {
        self.save_file_path.with_extension("download_state.json")
    }

    /// Save state to disk
    pub async fn save(&self) -> Result<()> {
        let state_path = self.state_file_path();
        let json = serde_json::to_string_pretty(self)?;
        tokio::fs::write(&state_path, json).await?;
        Ok(())
    }

    /// Load state from disk
    pub async fn load(state_path: &Path) -> Result<Self> {
        let json = tokio::fs::read_to_string(state_path).await?;
        let state: Self = serde_json::from_str(&json)?;
        Ok(state)
    }

    /// Delete state file
    pub async fn delete(&self) -> Result<()> {
        let state_path = self.state_file_path();
        if state_path.exists() {
            tokio::fs::remove_file(&state_path).await?;
        }
        Ok(())
    }
}

/// Resumable HTTP file downloader
///
/// Port of C#'s NetworkFileStream class (AaxDecrypter/NetworkFileStream.cs)
pub struct ResumableStream {
    /// HTTP client
    client: Client,

    /// Stream state
    state: StreamState,

    /// Progress tracker
    progress_tracker: Option<ProgressTracker>,

    /// Retry configuration
    max_retries: u32,
}

impl ResumableStream {
    /// Create new resumable stream
    ///
    /// Based on NetworkFileStream constructor (lines 87-112)
    pub async fn new(
        url: String,
        output_path: PathBuf,
        request_headers: std::collections::HashMap<String, String>,
    ) -> Result<Self> {
        // Validate output directory exists
        if let Some(parent) = output_path.parent() {
            if !parent.exists() {
                return Err(LibationError::FileNotFound(
                    format!("Directory does not exist: {:?}", parent)
                ));
            }
        }

        let mut state = StreamState::new(url.clone(), output_path.clone());
        state.request_headers = request_headers;

        // Check if file exists and get its size for resume
        if output_path.exists() {
            let metadata = tokio::fs::metadata(&output_path).await?;
            state.write_position = metadata.len();
        }

        let client = Client::builder()
            .timeout(Duration::from_secs(300)) // 5 minute timeout
            .build()?;

        Ok(Self {
            client,
            state,
            progress_tracker: None,
            max_retries: MAX_RETRIES,
        })
    }

    /// Resume from saved state
    ///
    /// Based on NetworkFileStreamPersister (NetworkFileStreamPersister.cs)
    pub async fn from_state(state_path: &Path) -> Result<Self> {
        let state = StreamState::load(state_path).await?;

        // Validate file still exists
        if !state.save_file_path.exists() {
            return Err(LibationError::FileNotFound(
                "Download file no longer exists".to_string()
            ));
        }

        // Verify write position matches file size
        let metadata = tokio::fs::metadata(&state.save_file_path).await?;
        if metadata.len() != state.write_position {
            return Err(LibationError::InvalidData(
                format!(
                    "File size mismatch: expected {}, got {}",
                    state.write_position,
                    metadata.len()
                )
            ));
        }

        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()?;

        Ok(Self {
            client,
            state,
            progress_tracker: None,
            max_retries: MAX_RETRIES,
        })
    }

    /// Initialize progress tracking
    pub fn with_progress(&mut self, asin: String, title: String) {
        self.progress_tracker = Some(ProgressTracker::new(
            asin,
            title,
            self.state.content_length,
        ));
    }

    /// Download file with optional progress callback
    ///
    /// Port of NetworkFileStream.BeginDownloadingAsync and DownloadLoopInternal
    /// (lines 156-218)
    pub async fn download<F>(&mut self, mut progress_callback: F) -> Result<()>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        // If already complete, nothing to do
        if self.state.write_position == self.state.content_length
            && self.state.content_length > 0
        {
            if let Some(ref mut tracker) = self.progress_tracker {
                tracker.set_state(ProgressState::Completed);
                progress_callback(tracker.clone_progress());
            }
            return Ok(());
        }

        // Update progress state to Downloading
        if let Some(ref mut tracker) = self.progress_tracker {
            tracker.set_state(ProgressState::Downloading);
        }

        // Retry loop for connection drops
        let mut retries = 0;
        loop {
            match self.download_internal(&mut progress_callback).await {
                Ok(()) => {
                    // Success - delete state file and return
                    self.state.delete().await?;
                    if let Some(ref mut tracker) = self.progress_tracker {
                        tracker.set_state(ProgressState::Completed);
                        progress_callback(tracker.clone_progress());
                    }
                    return Ok(());
                }
                Err(e) => {
                    // Check if we should retry
                    if retries >= self.max_retries {
                        if let Some(ref mut tracker) = self.progress_tracker {
                            tracker.set_error(format!("Download failed after {} retries: {}", self.max_retries, e));
                            progress_callback(tracker.clone_progress());
                        }
                        return Err(e);
                    }

                    // Check if error is retryable
                    if self.is_retryable_error(&e) {
                        retries += 1;
                        let backoff = Duration::from_secs(2u64.pow(retries.min(5)));
                        tokio::time::sleep(backoff).await;

                        // Save current state before retry
                        self.state.save().await?;
                        continue;
                    } else {
                        // Non-retryable error
                        if let Some(ref mut tracker) = self.progress_tracker {
                            tracker.set_error(format!("Download failed: {}", e));
                            progress_callback(tracker.clone_progress());
                        }
                        return Err(e);
                    }
                }
            }
        }
    }

    /// Internal download implementation
    ///
    /// Based on DownloadToFile method (lines 252-318)
    async fn download_internal<F>(&mut self, progress_callback: &mut F) -> Result<()>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        // Request next byte range
        let response = self.request_next_byte_range().await?;

        // Open file for appending
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.state.save_file_path)
            .await?;

        let mut writer = BufWriter::with_capacity(DOWNLOAD_BUFF_SZ, file);

        // Get response stream
        let mut stream = response.bytes_stream();

        // Track bytes for periodic flush
        let mut bytes_since_flush = 0u64;
        let mut next_flush = self.state.write_position + DATA_FLUSH_SZ;

        // Download loop
        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result?;
            let chunk_len = chunk.len() as u64;

            // Write chunk to file
            writer.write_all(&chunk).await?;

            // Update position
            self.state.write_position += chunk_len;
            bytes_since_flush += chunk_len;

            // Flush periodically
            if self.state.write_position >= next_flush {
                writer.flush().await?;
                self.state.save().await?;
                next_flush = self.state.write_position + DATA_FLUSH_SZ;
                bytes_since_flush = 0;

                // Update progress
                if let Some(ref mut tracker) = self.progress_tracker {
                    if tracker.update(self.state.write_position) {
                        progress_callback(tracker.clone_progress());
                    }
                }
            }
        }

        // Final flush
        writer.flush().await?;
        self.state.save().await?;

        // Final progress update
        if let Some(ref mut tracker) = self.progress_tracker {
            tracker.force_update(self.state.write_position);
            progress_callback(tracker.clone_progress());
        }

        // Verify download completed
        if self.state.write_position < self.state.content_length {
            return Err(LibationError::DownloadFailed(format!(
                "Download incomplete: {}/{} bytes",
                self.state.write_position, self.state.content_length
            )));
        }

        Ok(())
    }

    /// Request next byte range from server
    ///
    /// Based on RequestNextByteRangeAsync (lines 220-244)
    async fn request_next_byte_range(&mut self) -> Result<reqwest::Response> {
        let mut request = self.client.get(&self.state.url);

        // Add custom headers
        for (key, value) in &self.state.request_headers {
            if key.to_lowercase() != "range" {
                request = request.header(key, value);
            }
        }

        // Add Range header for resume
        if self.state.write_position > 0 {
            request = request.header("Range", format!("bytes={}-", self.state.write_position));
        }

        let response = request.send().await?;

        // Handle response status
        match response.status() {
            StatusCode::OK => {
                // Full content (no resume support or starting from beginning)
                if self.state.write_position > 0 {
                    // Server doesn't support ranges, need to start over
                    return Err(LibationError::DownloadFailed(
                        "Server does not support range requests".to_string()
                    ));
                }

                // Get total content length
                if let Some(content_length) = response.content_length() {
                    self.state.content_length = content_length;
                    if let Some(ref mut tracker) = self.progress_tracker {
                        tracker.get_progress();
                    }
                } else {
                    return Err(LibationError::DownloadFailed("No content length in response".to_string()));
                }

                Ok(response)
            }
            StatusCode::PARTIAL_CONTENT => {
                // Successful range request (line 234)
                let content_range = response
                    .headers()
                    .get("content-range")
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| LibationError::DownloadFailed("No Content-Range header".to_string()))?;

                // Parse Content-Range: bytes 1000-1999/2000
                let total_size = content_range
                    .split('/')
                    .nth(1)
                    .and_then(|s| s.parse::<u64>().ok())
                    .ok_or_else(|| LibationError::DownloadFailed("Invalid Content-Range format".to_string()))?;

                // Verify total size matches
                if self.state.content_length > 0 && self.state.content_length != total_size {
                    return Err(LibationError::DownloadFailed(format!(
                        "Content length mismatch: expected {}, got {}",
                        self.state.content_length, total_size
                    )));
                }

                self.state.content_length = total_size;
                Ok(response)
            }
            StatusCode::RANGE_NOT_SATISFIABLE => {
                // Range not satisfiable - file may have changed
                Err(LibationError::DownloadFailed("Range not satisfiable - file may have changed".to_string()))
            }
            _ => {
                Err(LibationError::DownloadFailed(format!(
                    "Unexpected status code: {}",
                    response.status()
                )))
            }
        }
    }

    /// Check if error is retryable
    fn is_retryable_error(&self, error: &LibationError) -> bool {
        match error {
            LibationError::NetworkError { is_transient, .. } => *is_transient,
            LibationError::DownloadFailed(msg) => {
                // Retry on connection errors, not on client errors
                !msg.contains("404") && !msg.contains("403") && !msg.contains("401")
            }
            _ => false,
        }
    }

    /// Get current stream state
    pub fn get_state(&self) -> &StreamState {
        &self.state
    }
}

/// Convenience function to download a file with progress tracking
///
/// Port of the common download pattern from DownloadDecryptBook.cs
pub async fn download_to_file<F>(
    url: String,
    output_path: PathBuf,
    asin: String,
    title: String,
    request_headers: std::collections::HashMap<String, String>,
    mut progress_callback: F,
) -> Result<()>
where
    F: FnMut(DownloadProgress) + Send,
{
    // Check if state file exists for resume
    let state_path = output_path.with_extension("download_state.json");

    let mut stream = if state_path.exists() {
        ResumableStream::from_state(&state_path).await?
    } else {
        ResumableStream::new(url, output_path, request_headers).await?
    };

    stream.with_progress(asin, title);
    stream.download(progress_callback).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_state_serialization() {
        let state = StreamState::new(
            "https://example.com/file.aaxc".to_string(),
            PathBuf::from("/tmp/download.aaxc"),
        );

        let json = serde_json::to_string(&state).unwrap();
        let deserialized: StreamState = serde_json::from_str(&json).unwrap();

        assert_eq!(state.url, deserialized.url);
        assert_eq!(state.save_file_path, deserialized.save_file_path);
    }

    #[test]
    fn test_state_file_path() {
        let state = StreamState::new(
            "https://example.com/file.aaxc".to_string(),
            PathBuf::from("/tmp/download.aaxc"),
        );

        let state_path = state.state_file_path();
        assert_eq!(state_path, PathBuf::from("/tmp/download.download_state.json"));
    }
}

