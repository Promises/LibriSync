//! Download management and streaming
//!
//! This module handles downloading audiobook files from Audible's CDN.
//!
//! # Reference C# Sources
//! - `AaxDecrypter/AudiobookDownloadBase.cs` - Base class for all download types
//! - `AaxDecrypter/NetworkFileStream.cs` - HTTP streaming with resume support
//! - `AaxDecrypter/NetworkFileStreamPersister.cs` - Persistent download state
//! - `FileLiberator/DownloadDecryptBook.cs` - High-level download orchestration
//! - `FileLiberator/DownloadOptions.cs` - Download configuration

pub mod manager;
pub mod stream;
pub mod progress;

// Re-export commonly used types
pub use manager::DownloadManager;
pub use progress::DownloadProgress;
