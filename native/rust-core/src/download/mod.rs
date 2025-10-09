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


//! Resumable audiobook download manager
//!
//! # Reference C# Sources
//! - **`AaxDecrypter/NetworkFileStream.cs`** - Resumable HTTP downloader with throttling (lines 1-422)
//! - **`AaxDecrypter/AudiobookDownloadBase.cs`** - Download orchestration and progress reporting (lines 1-218)
//! - **`FileLiberator/DownloadDecryptBook.cs`** - Download+decrypt workflow (lines 1-512)
//! - **`FileLiberator/DownloadOptions.Factory.cs`** - License request and content URL resolution (lines 1-307)
//!
//! # Architecture
//!
//! This module provides a resumable HTTP downloader that can:
//! - Download large files with automatic resume on network interruption
//! - Persist download state to JSON for cross-session resume
//! - Report progress (bytes downloaded, percentage, time remaining)
//! - Throttle download speed for network-constrained environments
//! - Handle range requests for partial content
//!
//! ## Core Components
//!
//! ### NetworkFileStream (stream.rs)
//! Direct port of NetworkFileStream.cs - A simultaneous file downloader and reader that:
//! - Downloads file in background task
//! - Flushes data to disk periodically (every 1MB)
//! - Saves download state to JSON for resume
//! - Supports HTTP range requests for resume
//! - Provides Stream interface for reading while downloading
//!
//! ### DownloadManager (manager.rs)
//! High-level download orchestration that:
//! - Requests download license from Audible API
//! - Resolves content URLs and metadata
//! - Creates NetworkFileStream instances
//! - Tracks download progress
//! - Handles cancellation
//! - Reports progress via callbacks
//!
//! ## Download Flow
//!
//! 1. **License Request** - Get download voucher/license from API
//!    - Reference: DownloadOptions.Factory.cs:24-38 - InitiateDownloadAsync()
//!    - Calls GetDownloadLicenseAsync() for content license
//!    - Extracts download URL from ContentMetadata.ContentUrl.OfflineUrl
//!
//! 2. **Download Initiation** - Create NetworkFileStream
//!    - Reference: AudiobookDownloadBase.cs:178-216 - OpenNetworkFileStream()
//!    - Check for existing download state JSON
//!    - Resume from saved position if available
//!    - Update URL if expired (CDN URLs expire after 1 hour)
//!
//! 3. **Background Download** - Download in separate task
//!    - Reference: NetworkFileStream.cs:182-218 - DownloadLoopInternal()
//!    - Make HTTP request with Range header
//!    - Download in 8KB chunks
//!    - Flush to disk every 1MB
//!    - Update JSON state periodically
//!    - Reconnect on connection errors
//!
//! 4. **Progress Reporting** - Report status to UI
//!    - Reference: AudiobookDownloadBase.cs:89-121 - reportProgress()
//!    - Calculate average speed
//!    - Estimate time remaining
//!    - Report percentage complete
//!    - Update every 200ms
//!
//! 5. **Completion** - Finalize download
//!    - Flush remaining data
//!    - Delete state JSON
//!    - Close file handles
//!    - Report 100% complete

pub mod manager;
pub mod stream;
pub mod progress;

// Re-export commonly used types
pub use manager::DownloadManager;
pub use progress::DownloadProgress;
