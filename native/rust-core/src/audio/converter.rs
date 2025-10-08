//! Audio format conversion
//!
//! # Reference C# Sources
//! - `FileLiberator/ConvertToMp3.cs` - MP3 conversion with FFmpeg/LAME
//! - `FileLiberator/AudioDecodable.cs` - Format conversion orchestration
//!
//! # Conversion Strategies (from ConvertToMp3.cs)
//!
//! ## M4B to MP3 (AAC to MP3)
//! - Use FFmpeg with LAME encoder
//! - Command: `ffmpeg -i input.m4b -codec:a libmp3lame -q:a 2 output.mp3`
//! - Quality levels:
//!   - 0 = ~245 kbps (highest)
//!   - 2 = ~190 kbps (high, default)
//!   - 4 = ~165 kbps (medium)
//!   - 6 = ~130 kbps (low)
//!
//! ## Split by Chapter
//! - Use FFmpeg with chapter metadata
//! - Extract chapter list from source
//! - Generate multiple files: `{title} - Chapter {n}.mp3`
//! - Preserve metadata for each chapter
//!
//! ## Copy vs Re-encode
//! - If source and target are same codec: `-c:a copy` (fast, lossless)
//! - If different codecs: re-encode (slower, quality loss)
//! - AAX → M4B: Copy (both AAC)
//! - M4B → MP3: Re-encode (AAC → MP3)

use crate::audio::decoder::{AudioDecoder, AudioFormat};
use crate::error::{LibationError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

/// Conversion progress callback type
pub type ProgressCallback = Arc<dyn Fn(f32) + Send + Sync>;

/// Bitrate options for lossy encoding
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Bitrate {
    /// Constant bitrate in kbps
    Cbr(u32),
    /// Variable bitrate with quality (0-9 for MP3, lower is better)
    Vbr(u8),
}

impl Default for Bitrate {
    fn default() -> Self {
        Self::Vbr(2) // High quality VBR
    }
}

/// Audio conversion options
/// Based on ConvertToMp3.cs configuration
#[derive(Debug, Clone)]
pub struct ConversionOptions {
    /// Output format
    pub output_format: AudioFormat,

    /// Bitrate setting
    pub bitrate: Bitrate,

    /// Split into separate files by chapter
    pub split_by_chapter: bool,

    /// Preserve original metadata
    pub preserve_metadata: bool,

    /// Preserve chapter markers (if output format supports)
    pub preserve_chapters: bool,

    /// Overwrite existing output file
    pub overwrite_existing: bool,

    /// Downsample to mono (reduce file size)
    pub downsample_mono: bool,
}

impl Default for ConversionOptions {
    fn default() -> Self {
        Self {
            output_format: AudioFormat::M4b,
            bitrate: Bitrate::default(),
            split_by_chapter: false,
            preserve_metadata: true,
            preserve_chapters: true,
            overwrite_existing: false,
            downsample_mono: false,
        }
    }
}

/// Audio converter
/// Handles conversion between different audio formats using FFmpeg
pub struct AudioConverter {
    options: ConversionOptions,
}

impl AudioConverter {
    /// Create new converter with options
    pub fn new(options: ConversionOptions) -> Self {
        Self { options }
    }

    /// Convert audio file to specified format
    ///
    /// Based on ConvertToMp3.cs::ProcessAsync
    pub async fn convert(&self, input: &Path, output: &Path) -> Result<()> {
        self.convert_with_progress(input, output, Arc::new(|_| {}))
            .await
    }

    /// Convert with progress callback
    ///
    /// Based on ConvertToMp3.cs with ConversionProgressUpdate events
    pub async fn convert_with_progress(
        &self,
        input: &Path,
        output: &Path,
        progress_callback: ProgressCallback,
    ) -> Result<()> {
        // Validate input exists
        if !input.exists() {
            return Err(LibationError::FileNotFound(format!(
                "{}: Input file does not exist",
                input.display()
            )));
        }

        // Check if output already exists
        if output.exists() && !self.options.overwrite_existing {
            return Err(LibationError::FileAlreadyExists(
                output.to_string_lossy().to_string(),
            ));
        }

        // Detect input format
        let input_format = AudioDecoder::detect_format(input).await?;

        // Get duration for progress tracking
        let duration = AudioDecoder::get_duration(input).await?;

        // Check if conversion is needed
        if input_format == self.options.output_format && !self.needs_processing() {
            // Just copy the file
            tokio::fs::copy(input, output).await.map_err(|e| {
                LibationError::FileIoError(format!("copy: {} - {}", output.display(), e))
            })?;
            progress_callback(1.0);
            return Ok(());
        }

        // Build FFmpeg command
        let command = self.build_ffmpeg_command(input, output, input_format)?;

        // Execute conversion with progress tracking
        self.execute_conversion(&command, duration, progress_callback)
            .await?;

        // Verify output was created
        if !output.exists() {
            return Err(LibationError::ConversionFailed(
                "Output file was not created".to_string(),
            ));
        }

        Ok(())
    }

    /// Split audio by chapters
    ///
    /// Based on ConvertToMp3.cs multi-part conversion
    pub async fn split_by_chapters(
        &self,
        input: &Path,
        output_dir: &Path,
    ) -> Result<Vec<PathBuf>> {
        // Create output directory if it doesn't exist
        tokio::fs::create_dir_all(output_dir).await.map_err(|e| {
            LibationError::FileIoError(format!(
                "create_dir: {} - {}",
                output_dir.display(),
                e
            ))
        })?;

        // Extract chapters using metadata module
        // This would typically use ChapterEditor::extract_chapters
        // For now, we'll use FFprobe to get chapters
        let chapters = Self::extract_chapters_for_splitting(input).await?;

        if chapters.is_empty() {
            return Err(LibationError::ConversionFailed(
                "No chapters found in file".to_string(),
            ));
        }

        let mut output_files = Vec::new();
        let input_stem = input
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("audiobook");
        let ext = self.options.output_format.to_extension();

        // Convert each chapter
        for (idx, chapter) in chapters.iter().enumerate() {
            let chapter_num = idx + 1;
            let output_filename = format!("{} - Chapter {:02}.{}", input_stem, chapter_num, ext);
            let output_path = output_dir.join(output_filename);

            // Build chapter-specific FFmpeg command
            let command = self.build_chapter_split_command(
                input,
                &output_path,
                &chapter.start_time,
                chapter.duration.as_deref(),
            )?;

            // Execute conversion
            self.execute_conversion(&command, chapter.duration_seconds, Arc::new(|_| {}))
                .await?;

            output_files.push(output_path);
        }

        Ok(output_files)
    }

    /// Build FFmpeg command for conversion
    ///
    /// Based on ConvertToMp3.cs FFmpeg command construction
    fn build_ffmpeg_command(
        &self,
        input: &Path,
        output: &Path,
        input_format: AudioFormat,
    ) -> Result<Vec<String>> {
        let mut cmd = vec![
            "ffmpeg".to_string(),
            "-i".to_string(),
            input.to_string_lossy().to_string(),
        ];

        // Overwrite output file if requested
        if self.options.overwrite_existing {
            cmd.push("-y".to_string());
        }

        // Audio codec selection
        match self.options.output_format {
            AudioFormat::Mp3 => {
                cmd.push("-codec:a".to_string());
                cmd.push("libmp3lame".to_string());

                // Bitrate/quality settings
                match self.options.bitrate {
                    Bitrate::Vbr(quality) => {
                        cmd.push("-q:a".to_string());
                        cmd.push(quality.to_string());
                    }
                    Bitrate::Cbr(kbps) => {
                        cmd.push("-b:a".to_string());
                        cmd.push(format!("{}k", kbps));
                    }
                }

                // ID3v2 version for better compatibility
                cmd.push("-id3v2_version".to_string());
                cmd.push("3".to_string());
            }
            AudioFormat::M4b | AudioFormat::M4a => {
                // Check if we can copy without re-encoding
                if input_format.is_mp4_container() && !self.needs_processing() {
                    cmd.push("-codec:a".to_string());
                    cmd.push("copy".to_string());
                } else {
                    cmd.push("-codec:a".to_string());
                    cmd.push("aac".to_string());

                    // AAC doesn't support VBR quality, use CBR
                    let kbps = match self.options.bitrate {
                        Bitrate::Cbr(k) => k,
                        Bitrate::Vbr(q) => Self::vbr_quality_to_bitrate(q),
                    };
                    cmd.push("-b:a".to_string());
                    cmd.push(format!("{}k", kbps));
                }
            }
            _ => {
                return Err(LibationError::UnsupportedAudioFormat(format!(
                    "Unsupported output format: {:?}",
                    self.options.output_format
                )));
            }
        }

        // Mono downsampling
        if self.options.downsample_mono {
            cmd.push("-ac".to_string());
            cmd.push("1".to_string());
        }

        // Metadata preservation
        if self.options.preserve_metadata {
            cmd.push("-map_metadata".to_string());
            cmd.push("0".to_string());
        }

        // Chapter preservation (for formats that support it)
        if self.options.preserve_chapters && self.options.output_format.is_mp4_container() {
            cmd.push("-map_chapters".to_string());
            cmd.push("0".to_string());
        }

        // Remove video streams (cover art will be handled separately)
        cmd.push("-vn".to_string());

        // Output file
        cmd.push(output.to_string_lossy().to_string());

        Ok(cmd)
    }

    /// Build FFmpeg command for chapter splitting
    fn build_chapter_split_command(
        &self,
        input: &Path,
        output: &Path,
        start_time: &str,
        duration: Option<&str>,
    ) -> Result<Vec<String>> {
        let mut cmd = vec![
            "ffmpeg".to_string(),
            "-i".to_string(),
            input.to_string_lossy().to_string(),
        ];

        // Start time
        cmd.push("-ss".to_string());
        cmd.push(start_time.to_string());

        // Duration (if specified)
        if let Some(dur) = duration {
            cmd.push("-t".to_string());
            cmd.push(dur.to_string());
        }

        // Copy codec (fast splitting)
        cmd.push("-codec:a".to_string());
        cmd.push("copy".to_string());

        // Overwrite if needed
        if self.options.overwrite_existing {
            cmd.push("-y".to_string());
        }

        cmd.push(output.to_string_lossy().to_string());

        Ok(cmd)
    }

    /// Execute FFmpeg conversion with progress tracking
    ///
    /// Based on ConvertToMp3.cs::ConversionProgressUpdate
    async fn execute_conversion(
        &self,
        command: &[String],
        total_duration: f64,
        progress_callback: ProgressCallback,
    ) -> Result<()> {
        let mut child = Command::new(&command[0])
            .args(&command[1..])
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    LibationError::FfmpegNotFound
                } else {
                    LibationError::FfmpegError(format!("Failed to execute ffmpeg: {}", e))
                }
            })?;

        // Read stderr for progress
        let stderr = child.stderr.take().ok_or_else(|| {
            LibationError::FfmpegError("Failed to capture ffmpeg stderr".to_string())
        })?;

        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();

        let last_progress = Arc::new(Mutex::new(0.0f32));

        // Spawn task to read progress
        let progress_task = tokio::spawn({
            let progress_callback = progress_callback.clone();
            let last_progress = last_progress.clone();
            async move {
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(progress) =
                        Self::parse_ffmpeg_progress(&line, total_duration)
                    {
                        let mut last = last_progress.lock().await;
                        if (progress - *last).abs() > 0.01 {
                            // Update every 1%
                            *last = progress;
                            progress_callback(progress);
                        }
                    }
                }
            }
        });

        // Wait for FFmpeg to complete
        let status = child.wait().await.map_err(|e| {
            LibationError::FfmpegError(format!("FFmpeg process failed: {}", e))
        })?;

        // Wait for progress task to complete
        let _ = progress_task.await;

        if !status.success() {
            return Err(LibationError::ConversionFailed(format!(
                "FFmpeg exited with status: {}",
                status
            )));
        }

        // Final progress update
        progress_callback(1.0);

        Ok(())
    }

    /// Parse FFmpeg progress from stderr line
    ///
    /// FFmpeg outputs: "time=00:01:23.45 bitrate=64.0kbits/s"
    fn parse_ffmpeg_progress(line: &str, total_duration: f64) -> Option<f32> {
        // Look for "time=" in the line
        if let Some(time_start) = line.find("time=") {
            let time_str = &line[time_start + 5..];
            // Extract timestamp (HH:MM:SS.ss)
            let time_end = time_str.find(' ').unwrap_or(time_str.len());
            let timestamp = &time_str[..time_end];

            // Parse HH:MM:SS.ss
            if let Some(elapsed_seconds) = Self::parse_timestamp(timestamp) {
                if total_duration > 0.0 {
                    let progress = (elapsed_seconds / total_duration).min(1.0) as f32;
                    return Some(progress);
                }
            }
        }
        None
    }

    /// Parse timestamp in format HH:MM:SS.ss to seconds
    fn parse_timestamp(timestamp: &str) -> Option<f64> {
        let parts: Vec<&str> = timestamp.split(':').collect();
        if parts.len() == 3 {
            let hours: f64 = parts[0].parse().ok()?;
            let minutes: f64 = parts[1].parse().ok()?;
            let seconds: f64 = parts[2].parse().ok()?;
            Some(hours * 3600.0 + minutes * 60.0 + seconds)
        } else {
            None
        }
    }

    /// Check if any processing is needed beyond format change
    fn needs_processing(&self) -> bool {
        self.options.downsample_mono
    }

    /// Convert VBR quality (0-9) to approximate CBR bitrate for AAC
    fn vbr_quality_to_bitrate(quality: u8) -> u32 {
        match quality {
            0 => 320,
            1 => 256,
            2 => 192,
            3 => 160,
            4 => 128,
            5 => 112,
            6 => 96,
            7 => 80,
            8 => 64,
            _ => 48,
        }
    }

    /// Extract chapters for splitting (simplified)
    async fn extract_chapters_for_splitting(path: &Path) -> Result<Vec<ChapterInfo>> {
        // Use FFprobe to extract chapters
        let output = Command::new("ffprobe")
            .arg("-v")
            .arg("quiet")
            .arg("-print_format")
            .arg("json")
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
            return Err(LibationError::FfmpegError("FFprobe failed".to_string()));
        }

        let json = String::from_utf8_lossy(&output.stdout);
        let probe: ChapterProbe = serde_json::from_str(&json).map_err(|e| {
            LibationError::AudioFormatDetectionFailed(format!(
                "Failed to parse chapters: {}",
                e
            ))
        })?;

        let chapters = probe
            .chapters
            .unwrap_or_default()
            .into_iter()
            .map(|c| {
                let start_seconds = c.start_time.parse::<f64>().unwrap_or(0.0);
                let end_seconds = c.end_time.parse::<f64>().unwrap_or(0.0);
                let duration_seconds = end_seconds - start_seconds;

                ChapterInfo {
                    start_time: c.start_time,
                    duration: Some(duration_seconds.to_string()),
                    duration_seconds,
                }
            })
            .collect();

        Ok(chapters)
    }
}

/// Chapter information for splitting
#[derive(Debug)]
struct ChapterInfo {
    start_time: String,
    duration: Option<String>,
    duration_seconds: f64,
}

/// FFprobe chapter output
#[derive(Debug, Deserialize)]
struct ChapterProbe {
    chapters: Option<Vec<ChapterProbeEntry>>,
}

#[derive(Debug, Deserialize)]
struct ChapterProbeEntry {
    start_time: String,
    end_time: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_timestamp() {
        assert_eq!(
            AudioConverter::parse_timestamp("00:00:30.50"),
            Some(30.5)
        );
        assert_eq!(
            AudioConverter::parse_timestamp("00:01:23.45"),
            Some(83.45)
        );
        assert_eq!(
            AudioConverter::parse_timestamp("01:30:00.00"),
            Some(5400.0)
        );
    }

    #[test]
    fn test_parse_ffmpeg_progress() {
        let line = "frame=1234 fps=100 q=-0.0 size=1024kB time=00:01:23.45 bitrate=64.0kbits/s speed=2.0x";
        let progress = AudioConverter::parse_ffmpeg_progress(line, 600.0);
        assert!(progress.is_some());
        let p = progress.unwrap();
        assert!((p - 0.1391).abs() < 0.01); // ~83.45 / 600 = 0.139
    }

    #[test]
    fn test_vbr_quality_to_bitrate() {
        assert_eq!(AudioConverter::vbr_quality_to_bitrate(0), 320);
        assert_eq!(AudioConverter::vbr_quality_to_bitrate(2), 192);
        assert_eq!(AudioConverter::vbr_quality_to_bitrate(4), 128);
    }

    #[test]
    fn test_bitrate_default() {
        assert_eq!(Bitrate::default(), Bitrate::Vbr(2));
    }
}
