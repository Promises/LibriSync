//! Audio processing, conversion, and metadata
//!
//! This module handles audio format conversion, metadata embedding,
//! and chapter marker management.
//!
//! # Reference C# Sources
//! - `FileLiberator/AudioFormatDecoder.cs` - Format detection and decoding
//! - `FileLiberator/ConvertToMp3.cs` - MP3 conversion with FFmpeg/LAME
//! - `FileLiberator/AudioDecodable.cs` - Audio file operations
//! - `AaxDecrypter/MpegUtil.cs` - MPEG audio utilities
//! - `AaxDecrypter/Cue.cs` - Cue sheet generation
//!
//! # Module Organization
//!
//! ## decoder
//! Format detection and audio file inspection:
//! - `AudioFormat` - Supported formats (AAX, AAXC, M4B, MP3, M4A)
//! - `AudioDecoder` - Format detection from files or bytes
//! - `AudioInfo` - Detailed file information (codec, bitrate, duration, etc.)
//! - `Codec` - Audio codec types (AAC-LC, MP3, E-AC-3, AC-4)
//!
//! ## converter
//! Format conversion between audio types:
//! - `AudioConverter` - Main conversion engine
//! - `ConversionOptions` - Conversion settings (format, quality, chapters)
//! - `Bitrate` - VBR or CBR encoding options
//! - Progress tracking support
//! - Chapter-based splitting
//!
//! ## metadata
//! Metadata and chapter management:
//! - `AudioMetadata` - Book metadata (title, authors, narrators, etc.)
//! - `MetadataEditor` - Embed/extract metadata and cover art
//! - `Chapter` - Chapter marker structure
//! - `ChapterEditor` - Embed/extract chapters, generate cue sheets
//! - `SeriesInfo` - Series information
//!
//! # FFmpeg Integration
//!
//! This module requires FFmpeg and FFprobe to be installed and available in PATH:
//! - FFmpeg: Audio conversion, metadata embedding, cover art handling
//! - FFprobe: Format detection, metadata extraction, chapter reading
//!
//! ## Installation
//! - macOS: `brew install ffmpeg`
//! - Linux: `apt-get install ffmpeg` or `yum install ffmpeg`
//! - Windows: Download from https://ffmpeg.org/download.html
//!
//! ## Minimum Version
//! FFmpeg 4.0 or higher is recommended for full feature support.

pub mod converter;
pub mod decoder;
pub mod metadata;

// Re-export commonly used types for convenience
pub use converter::{AudioConverter, Bitrate, ConversionOptions, ProgressCallback};
pub use decoder::{AudioDecoder, AudioFormat, AudioInfo, Codec};
pub use metadata::{AudioMetadata, Chapter, ChapterEditor, MetadataEditor, SeriesInfo};
