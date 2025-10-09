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


//! Download progress tracking and reporting
//!
//! # Reference C# Sources
//! - **`Dinah.Core.Net.Http/DownloadProgress.cs`** - Progress event args structure
//! - **`AaxDecrypter/AudiobookDownloadBase.cs`** - Progress event handlers (lines 20-21, 141-143)
//! - **`FileLiberator/DownloadDecryptBook.cs`** - Progress callbacks (lines 146-147, 167)
//!
//! C# DownloadProgress properties:
//! ```csharp
//! public class DownloadProgress : EventArgs
//! {
//!     public long BytesReceived { get; set; }
//!     public long TotalBytesToReceive { get; set; }
//!     public double ProgressPercentage { get; set; }
//! }
//! ```

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

/// Download progress information
///
/// # Reference
/// C# type: `Dinah.Core.Net.Http.DownloadProgress`
/// Used in: AudiobookDownloadBase.cs:20 - DecryptProgressUpdate event
///
/// This structure is passed to progress callbacks to report download status.
/// Extended with book metadata for UI display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadProgress {
    /// Audible ASIN (book identifier)
    pub asin: String,

    /// Book title
    pub title: String,

    /// Number of bytes downloaded so far
    /// Reference: DownloadProgress.cs - BytesReceived property
    pub bytes_received: u64,

    /// Alias for bytes_received (used by manager.rs)
    #[serde(skip)]
    pub bytes_downloaded: u64,

    /// Total size of the file in bytes
    /// Reference: DownloadProgress.cs - TotalBytesToReceive property
    pub total_bytes: u64,

    /// Progress as a percentage (0.0 - 100.0)
    /// Reference: DownloadProgress.cs - ProgressPercentage property
    pub progress_percentage: f64,

    /// Alias for progress_percentage (used by manager.rs)
    #[serde(skip)]
    pub percent_complete: f64,

    /// Current download speed in bytes per second
    /// Calculated from recent download activity
    pub bytes_per_second: u64,

    /// Alias for bytes_per_second (used by manager.rs)
    #[serde(skip)]
    pub download_speed: f64,

    /// Estimated time remaining until completion
    /// Calculated from bytes remaining and current speed
    pub time_remaining: Option<Duration>,

    /// Estimated time remaining in seconds (used by manager.rs)
    #[serde(skip)]
    pub eta_seconds: u64,

    /// Current download state
    pub state: DownloadState,

    /// Error message if download failed
    pub error_message: Option<String>,
}

impl DownloadProgress {
    /// Create a new progress report
    pub fn new(asin: String, title: String, bytes_received: u64, total_bytes: u64) -> Self {
        let progress_percentage = if total_bytes > 0 {
            (bytes_received as f64 / total_bytes as f64) * 100.0
        } else {
            0.0
        };

        let eta_seconds = 0u64;

        Self {
            asin,
            title,
            bytes_received,
            bytes_downloaded: bytes_received,
            total_bytes,
            progress_percentage,
            percent_complete: progress_percentage,
            bytes_per_second: 0,
            download_speed: 0.0,
            time_remaining: None,
            eta_seconds,
            state: DownloadState::Pending,
            error_message: None,
        }
    }

    /// Update with speed and time remaining estimates
    pub fn with_estimates(mut self, bytes_per_second: u64) -> Self {
        self.bytes_per_second = bytes_per_second;
        self.download_speed = bytes_per_second as f64;

        if bytes_per_second > 0 && self.bytes_received < self.total_bytes {
            let bytes_remaining = self.total_bytes - self.bytes_received;
            let seconds_remaining = bytes_remaining / bytes_per_second;
            self.time_remaining = Some(Duration::from_secs(seconds_remaining));
            self.eta_seconds = seconds_remaining;
        }

        self
    }

    /// Update bytes received and recalculate percentages
    pub fn update_bytes(&mut self, bytes_received: u64) {
        self.bytes_received = bytes_received;
        self.bytes_downloaded = bytes_received;

        self.progress_percentage = if self.total_bytes > 0 {
            (bytes_received as f64 / self.total_bytes as f64) * 100.0
        } else {
            0.0
        };
        self.percent_complete = self.progress_percentage;
    }

    /// Set download state
    pub fn set_state(&mut self, state: DownloadState) {
        self.state = state;
    }

    /// Set error message and mark as failed
    pub fn set_error(&mut self, error: String) {
        self.state = DownloadState::Failed;
        self.error_message = Some(error);
    }

    /// Check if download is complete
    pub fn is_complete(&self) -> bool {
        self.bytes_received >= self.total_bytes && self.total_bytes > 0
    }

    /// Get progress as a fraction (0.0 - 1.0)
    pub fn as_fraction(&self) -> f64 {
        self.progress_percentage / 100.0
    }
}

impl Default for DownloadProgress {
    /// Create a zero progress (used at start and end)
    /// Reference: AudiobookDownloadBase.cs:53-58 - zeroProgress initialization
    fn default() -> Self {
        Self {
            asin: String::new(),
            title: String::new(),
            bytes_received: 0,
            bytes_downloaded: 0,
            total_bytes: 0,
            progress_percentage: 0.0,
            percent_complete: 0.0,
            bytes_per_second: 0,
            download_speed: 0.0,
            time_remaining: None,
            eta_seconds: 0,
            state: DownloadState::Pending,
            error_message: None,
        }
    }
}

/// Type alias for progress callback functions
///
/// # Reference
/// C# equivalent: `EventHandler<DownloadProgress>`
/// Used in: AudiobookDownloadBase.cs:20 - public event EventHandler<DownloadProgress>? DecryptProgressUpdate
///
/// Uses Arc instead of Box to enable Clone trait
pub type ProgressCallback = Arc<dyn Fn(DownloadProgress) + Send + Sync>;

/// Download state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DownloadState {
    /// Download is queued but not started
    Queued,
    /// Download is pending/waiting
    Pending,
    /// Download is in progress
    Downloading,
    /// Download is paused
    Paused,
    /// Download completed successfully
    Completed,
    /// Download failed with error
    Failed,
    /// Download was cancelled
    Cancelled,
}

/// Progress tracker for monitoring download state
pub struct ProgressTracker {
    /// Current state
    pub state: DownloadState,
    /// Latest progress report
    pub progress: DownloadProgress,
    /// Speed calculator
    speed_calc: AverageSpeed,
    /// Last update timestamp for throttling
    last_update: std::time::Instant,
    /// Minimum interval between updates (milliseconds)
    update_interval_ms: u64,
}

impl ProgressTracker {
    /// Create a new progress tracker with book metadata
    pub fn new(asin: String, title: String, total_bytes: u64) -> Self {
        Self {
            state: DownloadState::Pending,
            progress: DownloadProgress::new(asin, title, 0, total_bytes),
            speed_calc: AverageSpeed::new(),
            last_update: std::time::Instant::now(),
            update_interval_ms: 200, // Update every 200ms
        }
    }

    /// Update progress with new position
    /// Returns true if enough time has passed to trigger a callback
    pub fn update(&mut self, bytes_received: u64, total_bytes: u64) {
        self.speed_calc.add_position(bytes_received);
        let speed = self.speed_calc.average();

        self.progress.update_bytes(bytes_received);
        self.progress.total_bytes = total_bytes;
        self.progress = self.progress.clone().with_estimates(speed);
        self.progress.state = self.state;
    }

    /// Force an immediate progress update (returns true to trigger callback)
    pub fn force_update(&mut self, bytes_received: u64) {
        self.update(bytes_received, self.progress.total_bytes);
        self.last_update = std::time::Instant::now();
    }

    /// Check if enough time has passed to send an update
    pub fn should_update(&self) -> bool {
        self.last_update.elapsed().as_millis() >= self.update_interval_ms as u128
    }

    /// Get a clone of the current progress
    pub fn get_progress(&self) -> DownloadProgress {
        self.progress.clone()
    }

    /// Clone progress (alias for get_progress)
    pub fn clone_progress(&self) -> DownloadProgress {
        self.get_progress()
    }

    /// Set download state
    pub fn set_state(&mut self, state: DownloadState) {
        self.state = state;
        self.progress.set_state(state);
    }

    /// Set error message
    pub fn set_error(&mut self, error: String) {
        self.progress.set_error(error);
        self.state = DownloadState::Failed;
    }
}

impl Default for ProgressTracker {
    fn default() -> Self {
        Self::new(String::new(), String::new(), 0)
    }
}

/// Average speed calculator for smooth speed estimates
///
/// # Reference
/// C# type: `Dinah.Core.AverageSpeed` (external library)
/// Used in: AudiobookDownloadBase.cs:91 - AverageSpeed averageSpeed = new()
///
/// Tracks recent download positions to calculate average speed
pub struct AverageSpeed {
    /// Recent position samples
    positions: Vec<u64>,
    /// Timestamps for each sample
    timestamps: Vec<std::time::Instant>,
    /// Maximum number of samples to keep
    max_samples: usize,
}

impl AverageSpeed {
    /// Create a new speed tracker
    ///
    /// Reference: AudiobookDownloadBase.cs:91 - new AverageSpeed()
    pub fn new() -> Self {
        Self {
            positions: Vec::new(),
            timestamps: Vec::new(),
            max_samples: 10, // Keep last 10 samples (~2 seconds at 200ms intervals)
        }
    }

    /// Add a new position sample
    ///
    /// Reference: AudiobookDownloadBase.cs:99 - averageSpeed.AddPosition(InputFilePosition)
    pub fn add_position(&mut self, position: u64) {
        self.positions.push(position);
        self.timestamps.push(std::time::Instant::now());

        // Keep only recent samples
        if self.positions.len() > self.max_samples {
            self.positions.remove(0);
            self.timestamps.remove(0);
        }
    }

    /// Get average speed in bytes per second
    ///
    /// Reference: AudiobookDownloadBase.cs:101 - averageSpeed.Average
    pub fn average(&self) -> u64 {
        if self.positions.len() < 2 {
            return 0;
        }

        let first_pos = self.positions[0];
        let last_pos = *self.positions.last().unwrap();
        let first_time = self.timestamps[0];
        let last_time = *self.timestamps.last().unwrap();

        let bytes_diff = last_pos.saturating_sub(first_pos);
        let time_diff = last_time.duration_since(first_time);

        if time_diff.as_secs_f64() > 0.0 {
            (bytes_diff as f64 / time_diff.as_secs_f64()) as u64
        } else {
            0
        }
    }
}

impl Default for AverageSpeed {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_download_progress_new() {
        let progress = DownloadProgress::new(
            "B001".to_string(),
            "Test Book".to_string(),
            500,
            1000
        );
        assert_eq!(progress.asin, "B001");
        assert_eq!(progress.title, "Test Book");
        assert_eq!(progress.bytes_received, 500);
        assert_eq!(progress.bytes_downloaded, 500);
        assert_eq!(progress.total_bytes, 1000);
        assert_eq!(progress.progress_percentage, 50.0);
        assert_eq!(progress.percent_complete, 50.0);
    }

    #[test]
    fn test_download_progress_with_estimates() {
        let progress = DownloadProgress::new(
            "B001".to_string(),
            "Test Book".to_string(),
            500,
            1000
        ).with_estimates(100); // 100 bytes/sec

        assert_eq!(progress.bytes_per_second, 100);
        assert_eq!(progress.download_speed, 100.0);
        assert!(progress.time_remaining.is_some());
        assert_eq!(progress.time_remaining.unwrap().as_secs(), 5); // 500 bytes / 100 bps = 5 secs
        assert_eq!(progress.eta_seconds, 5);
    }

    #[test]
    fn test_download_progress_complete() {
        let progress = DownloadProgress::new(
            "B001".to_string(),
            "Test Book".to_string(),
            1000,
            1000
        );
        assert!(progress.is_complete());
        assert_eq!(progress.progress_percentage, 100.0);
    }

    #[test]
    fn test_average_speed() {
        let mut speed = AverageSpeed::new();

        speed.add_position(0);
        std::thread::sleep(std::time::Duration::from_millis(100));
        speed.add_position(1000);

        let avg = speed.average();
        // Should be around 10000 bytes/sec (1000 bytes in 0.1 seconds)
        assert!(avg > 8000 && avg < 12000, "Average speed was {}", avg);
    }
}
