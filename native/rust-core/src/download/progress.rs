//! Download progress tracking and reporting
//!
//! # Reference C# Sources
//! - `AaxDecrypter/AverageSpeed.cs` - Speed calculation with moving average
//! - `FileLiberator/Processable.cs` - Progress event handling
//! - `AaxDecrypter/AudiobookDownloadBase.cs` - Progress update patterns
//!
//! # Progress Information
//! - ASIN and title for identification
//! - Bytes downloaded / total bytes
//! - Current speed (MB/s) with moving average
//! - Time elapsed and remaining (estimated)
//! - Percentage complete
//! - Download state (Queued, Downloading, Paused, etc.)

use std::time::{Duration, Instant};
use std::collections::VecDeque;
use serde::{Deserialize, Serialize};

/// Download state enum representing the lifecycle of a download
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DownloadState {
    /// Download is queued but not started
    Queued,
    /// Currently downloading from server
    Downloading,
    /// Download paused by user
    Paused,
    /// Download completed successfully
    Completed,
    /// Download failed with error
    Failed,
    /// Download cancelled by user
    Cancelled,
}

/// Progress snapshot for a single download
///
/// Based on C#'s DownloadProgress event args (Dinah.Core.Net.Http)
/// and NetworkFileStream progress tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadProgress {
    /// Audible ASIN identifier
    pub asin: String,

    /// Book title for display
    pub title: String,

    /// Bytes downloaded so far
    pub bytes_downloaded: u64,

    /// Total bytes to download (0 if unknown)
    pub total_bytes: u64,

    /// Percentage complete (0.0 - 100.0)
    pub percent_complete: f64,

    /// Current download speed in bytes per second
    pub download_speed: f64,

    /// Estimated time remaining in seconds (0 if unknown)
    pub eta_seconds: u64,

    /// Current state of the download
    pub state: DownloadState,

    /// Optional error message if state is Failed
    pub error_message: Option<String>,
}

impl DownloadProgress {
    /// Create a new progress snapshot
    pub fn new(asin: String, title: String, total_bytes: u64) -> Self {
        Self {
            asin,
            title,
            bytes_downloaded: 0,
            total_bytes,
            percent_complete: 0.0,
            download_speed: 0.0,
            eta_seconds: 0,
            state: DownloadState::Queued,
            error_message: None,
        }
    }

    /// Calculate percentage from bytes
    pub fn calculate_percentage(&mut self) {
        if self.total_bytes > 0 {
            self.percent_complete = (self.bytes_downloaded as f64 / self.total_bytes as f64) * 100.0;
        } else {
            self.percent_complete = 0.0;
        }
    }

    /// Calculate ETA from speed and remaining bytes
    pub fn calculate_eta(&mut self) {
        if self.download_speed > 0.0 && self.total_bytes > 0 {
            let remaining_bytes = self.total_bytes.saturating_sub(self.bytes_downloaded);
            self.eta_seconds = (remaining_bytes as f64 / self.download_speed) as u64;
        } else {
            self.eta_seconds = 0;
        }
    }

    /// Format download speed as human-readable string (e.g., "2.5 MB/s")
    pub fn speed_string(&self) -> String {
        let mb_per_sec = self.download_speed / 1_000_000.0;
        format!("{:.1} MB/s", mb_per_sec)
    }

    /// Format ETA as human-readable string (e.g., "5m 30s")
    pub fn eta_string(&self) -> String {
        if self.eta_seconds == 0 {
            return "calculating...".to_string();
        }

        let hours = self.eta_seconds / 3600;
        let minutes = (self.eta_seconds % 3600) / 60;
        let seconds = self.eta_seconds % 60;

        if hours > 0 {
            format!("{}h {}m", hours, minutes)
        } else if minutes > 0 {
            format!("{}m {}s", minutes, seconds)
        } else {
            format!("{}s", seconds)
        }
    }

    /// Format bytes as human-readable string (e.g., "45.2 MB")
    pub fn bytes_string(bytes: u64) -> String {
        let mb = bytes as f64 / 1_000_000.0;
        format!("{:.1} MB", mb)
    }

    /// Format progress as display string
    pub fn display_string(&self) -> String {
        match self.state {
            DownloadState::Queued => {
                format!("{}: Queued", self.title)
            }
            DownloadState::Downloading => {
                format!(
                    "{}: {:.1}% ({} / {}) - {} - {}",
                    self.title,
                    self.percent_complete,
                    Self::bytes_string(self.bytes_downloaded),
                    Self::bytes_string(self.total_bytes),
                    self.speed_string(),
                    self.eta_string()
                )
            }
            DownloadState::Paused => {
                format!("{}: Paused at {:.1}%", self.title, self.percent_complete)
            }
            DownloadState::Completed => {
                format!("{}: Completed", self.title)
            }
            DownloadState::Failed => {
                format!("{}: Failed - {}", self.title,
                    self.error_message.as_deref().unwrap_or("Unknown error"))
            }
            DownloadState::Cancelled => {
                format!("{}: Cancelled", self.title)
            }
        }
    }
}

/// Callback type for progress updates
pub type ProgressCallback = std::sync::Arc<dyn Fn(DownloadProgress) + Send + Sync>;

/// Speed tracker with moving average
///
/// Port of C#'s AverageSpeed.cs with statistical analysis
/// Uses a sliding window approach to smooth out network fluctuations
#[derive(Debug)]
pub struct SpeedTracker {
    /// Samples within the time window
    samples: VecDeque<SpeedSample>,

    /// Time window for averaging (default 10 seconds)
    window_duration: Duration,

    /// Start time for tracking
    start_time: Instant,
}

#[derive(Debug, Clone)]
struct SpeedSample {
    /// Timestamp of this sample
    timestamp: Instant,

    /// Total bytes at this point in time
    position: u64,
}

impl SpeedTracker {
    /// Create new speed tracker with default 10-second window
    pub fn new() -> Self {
        Self::with_window(Duration::from_secs(10))
    }

    /// Create new speed tracker with custom window
    pub fn with_window(window_duration: Duration) -> Self {
        Self {
            samples: VecDeque::new(),
            window_duration,
            start_time: Instant::now(),
        }
    }

    /// Add a position sample (total bytes downloaded so far)
    ///
    /// Based on C#'s AverageSpeed.AddPosition()
    pub fn add_position(&mut self, position: u64) {
        let now = Instant::now();

        // Add new sample
        self.samples.push_back(SpeedSample {
            timestamp: now,
            position,
        });

        // Remove samples outside the window
        while let Some(sample) = self.samples.front() {
            if now.duration_since(sample.timestamp) > self.window_duration {
                self.samples.pop_front();
            } else {
                break;
            }
        }
    }

    /// Get current average speed in bytes per second
    ///
    /// Based on C#'s AverageSpeed.Average property
    pub fn average_speed(&self) -> f64 {
        if self.samples.len() < 2 {
            return 0.0;
        }

        let first = self.samples.front().unwrap();
        let last = self.samples.back().unwrap();

        let bytes_delta = last.position.saturating_sub(first.position);
        let time_delta = last.timestamp.duration_since(first.timestamp).as_secs_f64();

        if time_delta > 0.0 {
            bytes_delta as f64 / time_delta
        } else {
            0.0
        }
    }

    /// Estimate time remaining based on current speed
    pub fn estimate_time_remaining(&self, bytes_remaining: u64) -> Option<Duration> {
        let speed = self.average_speed();
        if speed > 0.0 {
            let seconds = bytes_remaining as f64 / speed;
            Some(Duration::from_secs_f64(seconds))
        } else {
            None
        }
    }

    /// Get elapsed time since start
    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }
}

impl Default for SpeedTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Progress tracker for managing download progress state
///
/// Combines progress snapshot with speed tracking
#[derive(Debug)]
pub struct ProgressTracker {
    /// Current progress snapshot
    progress: DownloadProgress,

    /// Speed tracker for moving average
    speed_tracker: SpeedTracker,

    /// Start time of download
    start_time: Instant,

    /// Last update time (for throttling)
    last_update: Instant,

    /// Minimum interval between progress callbacks (e.g., 200ms)
    update_interval: Duration,
}

impl ProgressTracker {
    /// Create new progress tracker
    pub fn new(asin: String, title: String, total_bytes: u64) -> Self {
        Self {
            progress: DownloadProgress::new(asin, title, total_bytes),
            speed_tracker: SpeedTracker::new(),
            start_time: Instant::now(),
            last_update: Instant::now(),
            update_interval: Duration::from_millis(200), // Update every 200ms max
        }
    }

    /// Update progress with new position
    ///
    /// Returns true if enough time has passed and callback should be invoked
    pub fn update(&mut self, bytes_downloaded: u64) -> bool {
        self.progress.bytes_downloaded = bytes_downloaded;
        self.speed_tracker.add_position(bytes_downloaded);

        // Update calculated fields
        self.progress.download_speed = self.speed_tracker.average_speed();
        self.progress.calculate_percentage();
        self.progress.calculate_eta();

        // Check if enough time has passed for an update
        let now = Instant::now();
        if now.duration_since(self.last_update) >= self.update_interval {
            self.last_update = now;
            true
        } else {
            false
        }
    }

    /// Force an update regardless of time interval
    pub fn force_update(&mut self, bytes_downloaded: u64) {
        self.update(bytes_downloaded);
        self.last_update = Instant::now();
    }

    /// Set the download state
    pub fn set_state(&mut self, state: DownloadState) {
        self.progress.state = state;
    }

    /// Set error message and state to Failed
    pub fn set_error(&mut self, message: String) {
        self.progress.state = DownloadState::Failed;
        self.progress.error_message = Some(message);
    }

    /// Get current progress snapshot
    pub fn get_progress(&self) -> &DownloadProgress {
        &self.progress
    }

    /// Get a cloned progress snapshot
    pub fn clone_progress(&self) -> DownloadProgress {
        self.progress.clone()
    }

    /// Get elapsed time since start
    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_progress_percentage() {
        let mut progress = DownloadProgress::new(
            "B01234567".to_string(),
            "Test Book".to_string(),
            1_000_000,
        );

        progress.bytes_downloaded = 250_000;
        progress.calculate_percentage();
        assert_eq!(progress.percent_complete, 25.0);

        progress.bytes_downloaded = 1_000_000;
        progress.calculate_percentage();
        assert_eq!(progress.percent_complete, 100.0);
    }

    #[test]
    fn test_speed_tracker() {
        let mut tracker = SpeedTracker::new();

        // Simulate downloading 1MB per second
        tracker.add_position(0);
        thread::sleep(Duration::from_millis(100));
        tracker.add_position(100_000); // 100KB in 100ms = 1MB/s

        let speed = tracker.average_speed();
        // Should be approximately 1_000_000 bytes/sec
        assert!(speed > 900_000.0 && speed < 1_100_000.0);
    }

    #[test]
    fn test_eta_calculation() {
        let mut progress = DownloadProgress::new(
            "B01234567".to_string(),
            "Test Book".to_string(),
            10_000_000,
        );

        progress.bytes_downloaded = 5_000_000;
        progress.download_speed = 1_000_000.0; // 1MB/s
        progress.calculate_eta();

        // 5MB remaining at 1MB/s = 5 seconds
        assert_eq!(progress.eta_seconds, 5);
    }
}
