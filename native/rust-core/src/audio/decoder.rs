//! Audio format detection and decoding
//!
//! # Reference C# Sources
//! - `FileLiberator/AudioFormatDecoder.cs` - Format detection and FFmpeg decoding
//! - `AaxDecrypter/MpegUtil.cs` - MPEG frame parsing
//!
//! # Supported Formats
//! - AAX: Encrypted M4B (AAC codec)
//! - AAXC: Encrypted MPEG-DASH (AAC codec, chunked)
//! - M4B: Unencrypted M4B (AAC codec)
//! - MP3: MPEG Audio Layer 3
//! - M4A: Unencrypted AAC
//!
//! # Format Detection Strategy
//! 1. Check file extension (.aax, .aaxc, .m4b, .mp3, .m4a)
//! 2. Read file header (magic bytes)
//! 3. Parse container format (MP4, MPEG)
//! 4. Detect codec (AAC, MP3, EC-3, AC-4)
//! 5. Check for encryption markers

use crate::error::{LibationError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// Audio format enum
/// Based on AudioFormatDecoder.cs format detection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioFormat {
    /// AAX - Audible's encrypted AAC format (legacy)
    Aax,
    /// AAXC - Audible's Widevine-encrypted format (current)
    Aaxc,
    /// M4B - Unencrypted M4B audiobook
    M4b,
    /// MP3 - MPEG Audio Layer 3
    Mp3,
    /// M4A - Unencrypted AAC audio
    M4a,
    /// Unknown or unsupported format
    Unknown,
}

impl AudioFormat {
    /// Detect format from file extension
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "aax" => Self::Aax,
            "aaxc" => Self::Aaxc,
            "m4b" => Self::M4b,
            "mp3" => Self::Mp3,
            "m4a" => Self::M4a,
            _ => Self::Unknown,
        }
    }

    /// Check if format is encrypted
    pub fn is_encrypted(&self) -> bool {
        matches!(self, Self::Aax | Self::Aaxc)
    }

    /// Get file extension for this format
    pub fn to_extension(&self) -> &'static str {
        match self {
            Self::Aax => "aax",
            Self::Aaxc => "aaxc",
            Self::M4b => "m4b",
            Self::Mp3 => "mp3",
            Self::M4a => "m4a",
            Self::Unknown => "bin",
        }
    }

    /// Check if format is an MP4 container
    pub fn is_mp4_container(&self) -> bool {
        matches!(self, Self::Aax | Self::Aaxc | Self::M4b | Self::M4a)
    }
}

/// Audio codec types
/// Based on AudioFormatDecoder.cs codec detection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Codec {
    /// AAC-LC (Low Complexity)
    AacLc,
    /// xHE-AAC (Extended High Efficiency)
    XheAac,
    /// E-AC-3 (Enhanced AC-3, Dolby Digital Plus)
    Ec3,
    /// AC-4 (Dolby AC-4)
    Ac4,
    /// MP3 (MPEG Audio Layer 3)
    Mp3,
    /// Unknown codec
    Unknown,
}

impl Codec {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AacLc => "AAC-LC",
            Self::XheAac => "xHE-AAC",
            Self::Ec3 => "E-AC-3",
            Self::Ac4 => "AC-4",
            Self::Mp3 => "MP3",
            Self::Unknown => "Unknown",
        }
    }
}

/// Detailed audio file information
/// Extracted via FFprobe
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioInfo {
    /// Format (M4B, MP3, etc.)
    pub format: AudioFormat,
    /// Duration in seconds
    pub duration_seconds: f64,
    /// Bitrate in bits per second
    pub bitrate_bps: u64,
    /// Audio codec
    pub codec: Codec,
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Number of audio channels
    pub channels: u32,
    /// File size in bytes
    pub file_size: u64,
    /// Whether file has chapter markers
    pub has_chapters: bool,
}

/// FFprobe JSON output structures
#[derive(Debug, Deserialize)]
struct FfprobeOutput {
    format: FfprobeFormat,
    streams: Vec<FfprobeStream>,
    chapters: Option<Vec<FfprobeChapter>>,
}

#[derive(Debug, Deserialize)]
struct FfprobeFormat {
    duration: Option<String>,
    bit_rate: Option<String>,
    size: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FfprobeStream {
    codec_type: String,
    codec_name: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct FfprobeChapter {
    id: i64,
    start_time: String,
    end_time: String,
}

/// Audio format decoder
pub struct AudioDecoder;

impl AudioDecoder {
    /// Detect audio format from file
    ///
    /// Strategy (from AudioFormatDecoder.cs):
    /// 1. Try extension first (fast path)
    /// 2. If unknown, read file header
    /// 3. Check magic bytes for MP4 or MP3
    pub async fn detect_format(path: &Path) -> Result<AudioFormat> {
        // Try extension first
        if let Some(ext) = path.extension() {
            let format = AudioFormat::from_extension(ext.to_string_lossy().as_ref());
            if format != AudioFormat::Unknown {
                return Ok(format);
            }
        }

        // Read file header to detect format
        Self::detect_format_from_file_header(path).await
    }

    /// Detect format from file header (magic bytes)
    async fn detect_format_from_file_header(path: &Path) -> Result<AudioFormat> {
        let mut file = File::open(path).await.map_err(|e| {
            LibationError::FileNotFound(format!("{}: {}", path.display(), e))
        })?;

        let mut header = vec![0u8; 12];
        file.read_exact(&mut header).await.map_err(|e| {
            LibationError::InvalidAudioFile(format!(
                "Failed to read file header: {}",
                e
            ))
        })?;

        Self::detect_format_from_bytes(&header)
    }

    /// Detect format from byte header
    ///
    /// Magic bytes:
    /// - MP4: "ftyp" at bytes 4-7 (after 4-byte size field)
    /// - MP3: 0xFF 0xFB (MPEG frame sync) or "ID3" (ID3v2 tag)
    pub fn detect_format_from_bytes(bytes: &[u8]) -> Result<AudioFormat> {
        if bytes.len() < 12 {
            return Err(LibationError::InvalidAudioFile(
                "File too small to detect format".to_string(),
            ));
        }

        // Check for MP4 container (M4B, M4A, AAX, AAXC)
        if bytes.len() >= 8 && &bytes[4..8] == b"ftyp" {
            // All are MP4 containers, differentiate by encryption/extension
            // Default to M4B (most common for audiobooks)
            return Ok(AudioFormat::M4b);
        }

        // Check for MP3
        // ID3v2 tag
        if bytes.len() >= 3 && &bytes[0..3] == b"ID3" {
            return Ok(AudioFormat::Mp3);
        }

        // MP3 frame sync (11 bits set: 0xFF 0xE0-0xFF)
        if bytes.len() >= 2 && bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0 {
            return Ok(AudioFormat::Mp3);
        }

        Ok(AudioFormat::Unknown)
    }

    /// Get detailed audio information using FFprobe
    ///
    /// Executes: ffprobe -v quiet -print_format json -show_format -show_streams -show_chapters {path}
    pub async fn get_audio_info(path: &Path) -> Result<AudioInfo> {
        // Check file exists
        let metadata = tokio::fs::metadata(path).await.map_err(|e| {
            LibationError::FileNotFound(format!("{}: {}", path.display(), e))
        })?;

        let file_size = metadata.len();

        // Detect format
        let format = Self::detect_format(path).await?;

        // Probe with FFprobe
        let probe_output = Self::probe_with_ffprobe(path).await?;

        // Parse output
        let info = Self::parse_ffprobe_output(&probe_output, format, file_size)?;

        Ok(info)
    }

    /// Execute FFprobe command
    async fn probe_with_ffprobe(path: &Path) -> Result<String> {
        let output = Command::new("ffprobe")
            .arg("-v")
            .arg("quiet")
            .arg("-print_format")
            .arg("json")
            .arg("-show_format")
            .arg("-show_streams")
            .arg("-show_chapters")
            .arg(path.as_os_str())
            .output()
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    LibationError::FfmpegNotFound
                } else {
                    LibationError::FfmpegError(format!("Failed to execute ffprobe: {}", e))
                }
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(LibationError::FfmpegError(format!(
                "FFprobe failed: {}",
                stderr
            )));
        }

        String::from_utf8(output.stdout).map_err(|e| {
            LibationError::FfmpegError(format!("FFprobe output is not valid UTF-8: {}", e))
        })
    }

    /// Parse FFprobe JSON output
    fn parse_ffprobe_output(
        json: &str,
        format: AudioFormat,
        file_size: u64,
    ) -> Result<AudioInfo> {
        let probe: FfprobeOutput = serde_json::from_str(json).map_err(|e| {
            LibationError::AudioFormatDetectionFailed(format!(
                "Failed to parse FFprobe output: {}",
                e
            ))
        })?;

        // Extract duration
        let duration_seconds = probe
            .format
            .duration
            .and_then(|d| d.parse::<f64>().ok())
            .unwrap_or(0.0);

        // Extract bitrate
        let bitrate_bps = probe
            .format
            .bit_rate
            .and_then(|b| b.parse::<u64>().ok())
            .unwrap_or(0);

        // Find audio stream
        let audio_stream = probe
            .streams
            .iter()
            .find(|s| s.codec_type == "audio")
            .ok_or_else(|| {
                LibationError::InvalidAudioFile("No audio stream found in file".to_string())
            })?;

        // Extract codec
        let codec = match audio_stream.codec_name.as_deref() {
            Some("aac") => Codec::AacLc,
            Some("mp3") => Codec::Mp3,
            Some("eac3") => Codec::Ec3,
            Some("ac4") => Codec::Ac4,
            _ => Codec::Unknown,
        };

        // Extract sample rate
        let sample_rate = audio_stream
            .sample_rate
            .as_ref()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);

        // Extract channels
        let channels = audio_stream.channels.unwrap_or(0);

        // Check for chapters
        let has_chapters = probe.chapters.map(|c| !c.is_empty()).unwrap_or(false);

        Ok(AudioInfo {
            format,
            duration_seconds,
            bitrate_bps,
            codec,
            sample_rate,
            channels,
            file_size,
            has_chapters,
        })
    }

    /// Check if file is a valid audio file
    pub async fn is_valid_audio_file(path: &Path) -> Result<bool> {
        match Self::get_audio_info(path).await {
            Ok(info) => Ok(info.duration_seconds > 0.0 && info.channels > 0),
            Err(_) => Ok(false),
        }
    }

    /// Get duration in seconds (quick check without full probe)
    pub async fn get_duration(path: &Path) -> Result<f64> {
        let output = Command::new("ffprobe")
            .arg("-v")
            .arg("quiet")
            .arg("-print_format")
            .arg("json")
            .arg("-show_format")
            .arg(path.as_os_str())
            .output()
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    LibationError::FfmpegNotFound
                } else {
                    LibationError::FfmpegError(format!("Failed to execute ffprobe: {}", e))
                }
            })?;

        if !output.status.success() {
            return Err(LibationError::FfmpegError("FFprobe failed".to_string()));
        }

        let json = String::from_utf8_lossy(&output.stdout);
        let probe: FfprobeOutput = serde_json::from_str(&json).map_err(|e| {
            LibationError::AudioFormatDetectionFailed(format!("Failed to parse output: {}", e))
        })?;

        probe
            .format
            .duration
            .and_then(|d| d.parse::<f64>().ok())
            .ok_or_else(|| {
                LibationError::AudioFormatDetectionFailed(
                    "No duration found in file".to_string(),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_from_extension() {
        assert_eq!(AudioFormat::from_extension("m4b"), AudioFormat::M4b);
        assert_eq!(AudioFormat::from_extension("M4B"), AudioFormat::M4b);
        assert_eq!(AudioFormat::from_extension("mp3"), AudioFormat::Mp3);
        assert_eq!(AudioFormat::from_extension("aax"), AudioFormat::Aax);
        assert_eq!(AudioFormat::from_extension("aaxc"), AudioFormat::Aaxc);
        assert_eq!(AudioFormat::from_extension("m4a"), AudioFormat::M4a);
        assert_eq!(AudioFormat::from_extension("xyz"), AudioFormat::Unknown);
    }

    #[test]
    fn test_format_encryption() {
        assert!(AudioFormat::Aax.is_encrypted());
        assert!(AudioFormat::Aaxc.is_encrypted());
        assert!(!AudioFormat::M4b.is_encrypted());
        assert!(!AudioFormat::Mp3.is_encrypted());
    }

    #[test]
    fn test_detect_format_from_bytes_mp4() {
        // MP4 header: size (4 bytes) + "ftyp"
        let mp4_header = b"\x00\x00\x00\x20ftypM4B ";
        assert_eq!(
            AudioDecoder::detect_format_from_bytes(mp4_header).unwrap(),
            AudioFormat::M4b
        );
    }

    #[test]
    fn test_detect_format_from_bytes_mp3_id3() {
        let mp3_id3_header = b"ID3\x03\x00\x00\x00\x00\x00\x00\x00\x00";
        assert_eq!(
            AudioDecoder::detect_format_from_bytes(mp3_id3_header).unwrap(),
            AudioFormat::Mp3
        );
    }

    #[test]
    fn test_detect_format_from_bytes_mp3_frame_sync() {
        let mp3_frame_header = b"\xFF\xFB\x90\x00\x00\x00\x00\x00\x00\x00\x00\x00";
        assert_eq!(
            AudioDecoder::detect_format_from_bytes(mp3_frame_header).unwrap(),
            AudioFormat::Mp3
        );
    }

    #[test]
    fn test_codec_display() {
        assert_eq!(Codec::AacLc.as_str(), "AAC-LC");
        assert_eq!(Codec::Mp3.as_str(), "MP3");
        assert_eq!(Codec::Ec3.as_str(), "E-AC-3");
    }
}
