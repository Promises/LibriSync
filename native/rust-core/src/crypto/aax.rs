//! AAX file decryption (legacy Audible format)
//!
//! # Reference C# Sources
//! - External dependency: FFmpeg with activation bytes
//! - `FileLiberator/AudioDecodable.cs` - High-level decryption orchestration
//! - `FileLiberator/ConvertToMp3.cs` - Conversion after decryption
//! - `AaxDecrypter/MultiConvertFileProperties.cs` - File handling
//!
//! # AAX Format Details
//! - Container: MP4 (M4B)
//! - Audio codec: AAC
//! - Encryption: AES-128 CBC mode
//! - Key derivation: Based on activation bytes
//! - File structure: Standard MP4 with encrypted mdat atom
//!
//! # Decryption Process (Libation's approach)
//! 1. Use activation bytes as key
//! 2. Call FFmpeg with `-activation_bytes` parameter
//! 3. FFmpeg command (from Libation):
//!    ```
//!    ffmpeg -activation_bytes <BYTES> -i input.aax -vn -c:a copy output.m4b
//!    ```
//! 4. `-vn`: No video (strip cover art, will re-add later)
//! 5. `-c:a copy`: Copy audio stream without re-encoding
//!
//! # Alternative: Native Rust Decryption
//! - Can implement AES decryption directly
//! - Would need to:
//!   1. Parse MP4 structure (use mp4parse crate)
//!   2. Locate encrypted mdat atom
//!   3. Derive AES key from activation bytes
//!   4. Decrypt audio data
//!   5. Write decrypted MP4
//! - FFmpeg approach is simpler and battle-tested

use crate::crypto::activation::{ActivationBytes, format_activation_bytes};
use crate::error::{LibationError, Result};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// AAX file decrypter using FFmpeg
///
/// # C# Reference
/// Similar functionality to FileLiberator/AudioDecodable.cs
///
/// # Example
/// ```no_run
/// use rust_core::crypto::aax::AaxDecrypter;
/// use rust_core::crypto::activation::ActivationBytes;
/// use std::path::Path;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let activation_bytes = ActivationBytes::from_hex("1CEB00DA")?;
/// let decrypter = AaxDecrypter::new(activation_bytes);
///
/// decrypter.decrypt_file(
///     Path::new("input.aax"),
///     Path::new("output.m4b")
/// ).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct AaxDecrypter {
    activation_bytes: ActivationBytes,
}

impl AaxDecrypter {
    /// Create a new AAX decrypter with the given activation bytes
    ///
    /// # Arguments
    /// * `activation_bytes` - The 4-byte activation key
    pub fn new(activation_bytes: ActivationBytes) -> Self {
        Self { activation_bytes }
    }

    /// Decrypt an AAX file to M4B format using FFmpeg
    ///
    /// # C# Reference
    /// Corresponds to the decryption logic in AaxcDownloadConvertBase.cs
    ///
    /// # Arguments
    /// * `input` - Path to the input AAX file
    /// * `output` - Path to the output M4B file
    ///
    /// # Errors
    /// - FfmpegNotFound if FFmpeg is not installed
    /// - InvalidActivationBytes if the activation bytes are incorrect
    /// - FileNotFound if the input file doesn't exist
    /// - FfmpegError for other FFmpeg errors
    ///
    /// # FFmpeg Command
    /// ```bash
    /// ffmpeg -activation_bytes <BYTES> -i input.aax -vn -c:a copy output.m4b
    /// ```
    pub async fn decrypt_file(&self, input: &Path, output: &Path) -> Result<()> {
        self.decrypt_with_progress(input, output, |_| {}).await
    }

    /// Decrypt an AAX file with progress tracking
    ///
    /// # C# Reference
    /// Similar to ConversionProgressUpdate in ConvertToMp3.cs
    ///
    /// # Arguments
    /// * `input` - Path to the input AAX file
    /// * `output` - Path to the output M4B file
    /// * `progress_callback` - Callback function receiving progress (0.0 to 1.0)
    ///
    /// # Errors
    /// Same as `decrypt_file`
    ///
    /// # Example
    /// ```no_run
    /// # use rust_core::crypto::aax::AaxDecrypter;
    /// # use rust_core::crypto::activation::ActivationBytes;
    /// # use std::path::Path;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let activation_bytes = ActivationBytes::from_hex("1CEB00DA")?;
    /// # let decrypter = AaxDecrypter::new(activation_bytes);
    /// decrypter.decrypt_with_progress(
    ///     Path::new("input.aax"),
    ///     Path::new("output.m4b"),
    ///     |progress| {
    ///         println!("Progress: {:.1}%", progress * 100.0);
    ///     }
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn decrypt_with_progress<F>(
        &self,
        input: &Path,
        output: &Path,
        progress_callback: F,
    ) -> Result<()>
    where
        F: Fn(f32) + Send + 'static,
    {
        // Check if FFmpeg is available
        check_ffmpeg_available().await?;

        // Validate input file exists
        if !input.exists() {
            return Err(LibationError::FileNotFound(input.display().to_string()));
        }

        // Build FFmpeg command
        let activation_hex = self.activation_bytes.to_hex();
        let mut cmd = build_ffmpeg_command(input, output, &activation_hex)?;

        // Execute FFmpeg with progress tracking
        execute_ffmpeg(&mut cmd, progress_callback).await
    }

    /// Get the activation bytes as a hex string
    pub fn activation_bytes_hex(&self) -> String {
        self.activation_bytes.to_hex()
    }
}

/// Check if FFmpeg is available on the system
///
/// # Errors
/// - FfmpegNotFound if FFmpeg is not in PATH
async fn check_ffmpeg_available() -> Result<()> {
    match Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
    {
        Ok(status) if status.success() => Ok(()),
        Ok(_) => Err(LibationError::FfmpegNotFound),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(LibationError::FfmpegNotFound)
        }
        Err(e) => Err(LibationError::FfmpegError(format!(
            "Failed to check FFmpeg availability: {}",
            e
        ))),
    }
}

/// Build FFmpeg command for AAX decryption
///
/// # C# Reference
/// Corresponds to the FFmpeg command building in Libation
///
/// # Arguments
/// * `input` - Input AAX file path
/// * `output` - Output M4B file path
/// * `activation_bytes` - Activation bytes as hex string (8 characters)
///
/// # Returns
/// Configured Command ready to execute
fn build_ffmpeg_command(
    input: &Path,
    output: &Path,
    activation_bytes: &str,
) -> Result<Command> {
    let mut cmd = Command::new("ffmpeg");

    cmd
        // Overwrite output file if it exists
        .arg("-y")
        // Set activation bytes for decryption
        .arg("-activation_bytes")
        .arg(activation_bytes)
        // Input file
        .arg("-i")
        .arg(input)
        // No video (strip cover art - can be re-added later)
        .arg("-vn")
        // Copy audio codec without re-encoding (fast)
        .arg("-c:a")
        .arg("copy")
        // Output file
        .arg(output)
        // Capture stderr for progress parsing
        .stderr(Stdio::piped())
        .stdout(Stdio::null());

    Ok(cmd)
}

/// Execute FFmpeg command and parse progress
///
/// # C# Reference
/// Similar to progress tracking in ConvertToMp3.cs
///
/// # Arguments
/// * `cmd` - FFmpeg command to execute
/// * `progress_callback` - Callback to report progress (0.0 to 1.0)
///
/// # Errors
/// - FfmpegError if FFmpeg fails
/// - InvalidActivationBytes if activation bytes are wrong
///
/// # FFmpeg Progress Format
/// FFmpeg outputs progress on stderr in this format:
/// ```text
/// frame=  123 fps= 45 q=-1.0 size=   12345kB time=00:12:34.56 bitrate= 123.4kbits/s speed=45.6x
/// ```
async fn execute_ffmpeg<F>(cmd: &mut Command, progress_callback: F) -> Result<()>
where
    F: Fn(f32) + Send + 'static,
{
    let mut child = cmd.spawn().map_err(|e| {
        LibationError::FfmpegError(format!("Failed to spawn FFmpeg process: {}", e))
    })?;

    // Capture stderr for progress parsing
    let stderr = child.stderr.take().ok_or_else(|| {
        LibationError::FfmpegError("Failed to capture FFmpeg stderr".to_string())
    })?;

    let mut reader = BufReader::new(stderr).lines();
    let mut error_output = String::new();
    let mut duration_seconds: Option<f32> = None;

    // Read FFmpeg output line by line
    while let Some(line) = reader.next_line().await.map_err(|e| {
        LibationError::FfmpegError(format!("Failed to read FFmpeg output: {}", e))
    })? {
        // Accumulate error output for debugging
        error_output.push_str(&line);
        error_output.push('\n');

        // Parse duration from FFmpeg output (appears early in output)
        // Format: "Duration: 01:23:45.67, start: 0.000000, bitrate: 64 kb/s"
        if duration_seconds.is_none() {
            if let Some(duration) = parse_duration_from_line(&line) {
                duration_seconds = Some(duration);
            }
        }

        // Parse progress from FFmpeg output
        // Format: "time=00:12:34.56"
        if let Some(elapsed) = parse_time_from_line(&line) {
            if let Some(total) = duration_seconds {
                let progress = (elapsed / total).min(1.0).max(0.0);
                progress_callback(progress);
            }
        }
    }

    // Wait for FFmpeg to complete
    let status = child.wait().await.map_err(|e| {
        LibationError::FfmpegError(format!("Failed to wait for FFmpeg process: {}", e))
    })?;

    // Check exit status
    if !status.success() {
        // Check for specific error patterns
        if error_output.contains("Invalid data found when processing input")
            || error_output.contains("activation_bytes")
        {
            return Err(LibationError::InvalidActivationBytes(
                "FFmpeg failed to decrypt AAX file. The activation bytes may be incorrect."
                    .to_string(),
            ));
        }

        return Err(LibationError::FfmpegError(format!(
            "FFmpeg exited with status {}. Error output:\n{}",
            status.code().unwrap_or(-1),
            error_output
        )));
    }

    // Report 100% completion
    progress_callback(1.0);

    Ok(())
}

/// Parse duration from FFmpeg output line
///
/// # Format
/// ```text
/// Duration: 01:23:45.67, start: 0.000000, bitrate: 64 kb/s
/// ```
///
/// # Returns
/// Duration in seconds, or None if not found
fn parse_duration_from_line(line: &str) -> Option<f32> {
    if !line.contains("Duration:") {
        return None;
    }

    // Find the duration timestamp (HH:MM:SS.mm)
    let parts: Vec<&str> = line.split(',').collect();
    for part in parts {
        if part.contains("Duration:") {
            let duration_str = part.trim().strip_prefix("Duration:")?.trim();
            return parse_timestamp(duration_str);
        }
    }

    None
}

/// Parse time from FFmpeg progress line
///
/// # Format
/// ```text
/// frame=  123 fps= 45 q=-1.0 size=   12345kB time=00:12:34.56 bitrate= 123.4kbits/s speed=45.6x
/// ```
///
/// # Returns
/// Elapsed time in seconds, or None if not found
fn parse_time_from_line(line: &str) -> Option<f32> {
    // Look for "time=" pattern
    for part in line.split_whitespace() {
        if let Some(time_str) = part.strip_prefix("time=") {
            return parse_timestamp(time_str);
        }
    }

    None
}

/// Parse timestamp in HH:MM:SS.mm format to seconds
///
/// # Arguments
/// * `timestamp` - Timestamp string (e.g., "01:23:45.67")
///
/// # Returns
/// Total seconds as f32, or None if parsing fails
fn parse_timestamp(timestamp: &str) -> Option<f32> {
    let parts: Vec<&str> = timestamp.split(':').collect();
    if parts.len() != 3 {
        return None;
    }

    let hours: f32 = parts[0].parse().ok()?;
    let minutes: f32 = parts[1].parse().ok()?;
    let seconds: f32 = parts[2].parse().ok()?;

    Some(hours * 3600.0 + minutes * 60.0 + seconds)
}

/// Verify activation bytes by attempting to decrypt a small portion of the file
///
/// # Arguments
/// * `file` - Path to the AAX file
/// * `activation_bytes` - Activation bytes to verify
///
/// # Returns
/// - Ok(true) if activation bytes are valid
/// - Ok(false) if activation bytes are invalid
/// - Err if verification cannot be performed
///
/// # Note
/// This function is not yet implemented. It would require:
/// 1. Creating a temporary output file
/// 2. Running FFmpeg with -t 5 to decrypt only first 5 seconds
/// 3. Checking if output is valid
/// 4. Cleaning up temporary file
pub async fn verify_activation_bytes(file: &Path, activation_bytes: &ActivationBytes) -> Result<bool> {
    // Create temporary directory for test output
    let temp_dir = std::env::temp_dir();
    let temp_output = temp_dir.join(format!("aax_verify_{}.m4b", std::process::id()));

    // Create decrypter
    let decrypter = AaxDecrypter::new(*activation_bytes);

    // Build FFmpeg command with duration limit (only decrypt first 5 seconds)
    let activation_hex = activation_bytes.to_hex();
    let mut cmd = Command::new("ffmpeg");

    cmd.arg("-y")
        .arg("-activation_bytes")
        .arg(&activation_hex)
        .arg("-i")
        .arg(file)
        .arg("-t")
        .arg("5") // Only process first 5 seconds
        .arg("-vn")
        .arg("-c:a")
        .arg("copy")
        .arg(&temp_output)
        .stderr(Stdio::piped())
        .stdout(Stdio::null());

    // Execute FFmpeg
    let result = execute_ffmpeg(&mut cmd, |_| {}).await;

    // Clean up temporary file
    let _ = tokio::fs::remove_file(&temp_output).await;

    match result {
        Ok(_) => Ok(true),
        Err(LibationError::InvalidActivationBytes(_)) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Check if a file is a valid AAX file
///
/// # Arguments
/// * `path` - Path to the file to check
///
/// # Returns
/// - Ok(true) if the file is a valid AAX file
/// - Ok(false) if the file is not an AAX file
/// - Err if the file cannot be read
///
/// # Implementation
/// Checks for:
/// 1. File extension is .aax
/// 2. File has valid MP4/M4B signature
pub async fn is_aax_file(path: &Path) -> Result<bool> {
    // Check file extension
    if let Some(ext) = path.extension() {
        if ext.eq_ignore_ascii_case("aax") {
            // Check if file exists and is readable
            if path.exists() {
                // Read first 8 bytes to check for MP4 signature
                match tokio::fs::read(path).await {
                    Ok(data) if data.len() >= 8 => {
                        // MP4 files typically start with ftyp box
                        // Bytes 4-7 should be "ftyp"
                        Ok(data.len() >= 8 && &data[4..8] == b"ftyp")
                    }
                    Ok(_) => Ok(false), // File too small
                    Err(e) => Err(LibationError::FileIoError(format!(
                        "Failed to read file: {}",
                        e
                    ))),
                }
            } else {
                Err(LibationError::FileNotFound(path.display().to_string()))
            }
        } else {
            Ok(false)
        }
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_timestamp() {
        assert_eq!(parse_timestamp("00:00:01.50"), Some(1.5));
        assert_eq!(parse_timestamp("00:01:00.00"), Some(60.0));
        assert_eq!(parse_timestamp("01:00:00.00"), Some(3600.0));
        assert_eq!(parse_timestamp("01:23:45.67"), Some(5025.67));
    }

    #[test]
    fn test_parse_timestamp_invalid() {
        assert_eq!(parse_timestamp("invalid"), None);
        assert_eq!(parse_timestamp("00:00"), None);
        assert_eq!(parse_timestamp("00:00:00:00"), None);
    }

    #[test]
    fn test_parse_time_from_line() {
        let line = "frame=  123 fps= 45 q=-1.0 size=   12345kB time=00:12:34.56 bitrate= 123.4kbits/s speed=45.6x";
        assert_eq!(parse_time_from_line(line), Some(754.56));
    }

    #[test]
    fn test_parse_time_from_line_no_time() {
        let line = "frame=  123 fps= 45 q=-1.0 size=   12345kB bitrate= 123.4kbits/s speed=45.6x";
        assert_eq!(parse_time_from_line(line), None);
    }

    #[test]
    fn test_parse_duration_from_line() {
        let line = "  Duration: 01:23:45.67, start: 0.000000, bitrate: 64 kb/s";
        assert_eq!(parse_duration_from_line(line), Some(5025.67));
    }

    #[test]
    fn test_parse_duration_from_line_no_duration() {
        let line = "frame=  123 fps= 45 q=-1.0 size=   12345kB time=00:12:34.56 bitrate= 123.4kbits/s";
        assert_eq!(parse_duration_from_line(line), None);
    }

    #[test]
    fn test_build_ffmpeg_command() {
        let input = PathBuf::from("input.aax");
        let output = PathBuf::from("output.m4b");
        let activation_bytes = "1CEB00DA";

        let mut cmd = build_ffmpeg_command(&input, &output, activation_bytes).unwrap();

        // Command should be constructed properly
        // We can't easily test the full command here, but we can verify it doesn't panic
        assert!(format!("{:?}", cmd).contains("ffmpeg"));
    }

    #[test]
    fn test_aax_decrypter_creation() {
        let bytes = ActivationBytes::from_hex("1CEB00DA").unwrap();
        let decrypter = AaxDecrypter::new(bytes);

        assert_eq!(decrypter.activation_bytes_hex(), "1CEB00DA");
    }
}
