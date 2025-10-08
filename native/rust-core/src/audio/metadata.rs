//! Audio metadata and chapter management
//!
//! # Reference C# Sources
//! - `FileLiberator/AudioDecodable.cs` - Metadata embedding
//! - `AaxDecrypter/Cue.cs` - Cue sheet generation
//! - External: `AudibleApi/Common/Item.cs` - Metadata from API
//!
//! # Metadata Fields (from Audible API)
//! - Title, Authors, Narrators
//! - Publisher, Publication date, Language
//! - Series (title and position)
//! - Description, Rating, Genres/categories
//! - Cover art URL, Runtime
//!
//! # Metadata Embedding Strategy
//! - Use FFmpeg -metadata flag for standard tags
//! - Standard tags: title, artist (author), album, album_artist (narrator)
//! - date (publication date), genre, comment (description)
//! - Custom tags for series, ASIN, etc.
//! - Cover art: Embedded as attached_pic stream
//!
//! # Chapter Markers
//! - Stored in MP4/M4B: Chapter atom
//! - Stored in MP3: ID3v2 CHAP frames
//! - Format: [(title, start_ms, end_ms)]

use crate::error::{LibationError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Audio metadata structure
/// Based on Libation's book metadata fields
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioMetadata {
    pub title: String,
    pub authors: Vec<String>,
    pub narrators: Vec<String>,
    pub publisher: Option<String>,
    pub publication_date: Option<String>,
    pub language: Option<String>,
    pub series: Option<SeriesInfo>,
    pub description: Option<String>,
    pub genres: Vec<String>,
    pub runtime_minutes: Option<i32>,
    pub asin: Option<String>,
    pub cover_art_url: Option<String>,
}

impl AudioMetadata {
    /// Format authors for display: "Author1, Author2"
    pub fn format_authors(&self) -> String {
        self.authors.join(", ")
    }

    /// Format narrators for display: "Narrator1, Narrator2"
    pub fn format_narrators(&self) -> String {
        self.narrators.join(", ")
    }

    /// Format series for display: "Series Name #1"
    pub fn format_series(&self) -> Option<String> {
        self.series.as_ref().map(|s| {
            if let Some(pos) = &s.position {
                format!("{} #{}", s.name, pos)
            } else {
                s.name.clone()
            }
        })
    }
}

/// Series information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeriesInfo {
    pub name: String,
    /// Position in series (e.g., "1", "2.5", "1-3")
    pub position: Option<String>,
}

/// Chapter marker structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
    pub title: String,
    /// Start time in milliseconds
    pub start_ms: i64,
    /// End time in milliseconds
    pub end_ms: i64,
}

impl Chapter {
    /// Get chapter duration in milliseconds
    pub fn duration_ms(&self) -> i64 {
        self.end_ms - self.start_ms
    }

    /// Format timestamp for cue sheet (MM:SS:FF)
    /// FF = frames, 75 frames per second
    pub fn format_cue_timestamp(ms: i64) -> String {
        let total_seconds = ms / 1000;
        let frames = ((ms % 1000) * 75) / 1000;
        let minutes = total_seconds / 60;
        let seconds = total_seconds % 60;
        format!("{:02}:{:02}:{:02}", minutes, seconds, frames)
    }

    /// Format timestamp for FFmpeg (HH:MM:SS.mmm)
    pub fn format_ffmpeg_timestamp(ms: i64) -> String {
        let total_seconds = ms / 1000;
        let milliseconds = ms % 1000;
        let hours = total_seconds / 3600;
        let minutes = (total_seconds % 3600) / 60;
        let seconds = total_seconds % 60;
        format!(
            "{:02}:{:02}:{:02}.{:03}",
            hours, minutes, seconds, milliseconds
        )
    }
}

/// Metadata editor for embedding and extracting metadata
pub struct MetadataEditor;

impl MetadataEditor {
    /// Embed metadata into audio file
    ///
    /// Based on AudioDecodable.cs metadata writing
    pub async fn embed_metadata(file: &Path, metadata: &AudioMetadata) -> Result<()> {
        // Create temporary file for output
        let temp_file = file.with_extension("tmp.m4b");

        // Build FFmpeg command
        let mut cmd = vec![
            "ffmpeg".to_string(),
            "-i".to_string(),
            file.to_string_lossy().to_string(),
            "-codec".to_string(),
            "copy".to_string(),
        ];

        // Add metadata flags
        cmd.push("-metadata".to_string());
        cmd.push(format!("title={}", metadata.title));

        if !metadata.authors.is_empty() {
            cmd.push("-metadata".to_string());
            cmd.push(format!("artist={}", metadata.format_authors()));
        }

        cmd.push("-metadata".to_string());
        cmd.push(format!("album={}", metadata.title));

        if !metadata.narrators.is_empty() {
            cmd.push("-metadata".to_string());
            cmd.push(format!("album_artist={}", metadata.format_narrators()));
        }

        if let Some(publisher) = &metadata.publisher {
            cmd.push("-metadata".to_string());
            cmd.push(format!("publisher={}", publisher));
        }

        if let Some(date) = &metadata.publication_date {
            cmd.push("-metadata".to_string());
            cmd.push(format!("date={}", date));
        }

        if !metadata.genres.is_empty() {
            cmd.push("-metadata".to_string());
            cmd.push(format!("genre={}", metadata.genres.join("; ")));
        }

        if let Some(description) = &metadata.description {
            cmd.push("-metadata".to_string());
            cmd.push(format!("comment={}", description));
        }

        if let Some(series) = metadata.format_series() {
            cmd.push("-metadata".to_string());
            cmd.push(format!("series={}", series));
        }

        if let Some(asin) = &metadata.asin {
            cmd.push("-metadata".to_string());
            cmd.push(format!("asin={}", asin));
        }

        // Overwrite temp file
        cmd.push("-y".to_string());
        cmd.push(temp_file.to_string_lossy().to_string());

        // Execute FFmpeg
        Self::execute_ffmpeg(&cmd).await?;

        // Replace original with temp file
        fs::rename(&temp_file, file).await.map_err(|e| {
            LibationError::FileIoError(format!("{}: {} - {}", "rename".to_string(), file.to_string_lossy().to_string(), e.to_string(),
            ))
        })?;

        Ok(())
    }

    /// Extract metadata from audio file
    ///
    /// Uses FFprobe to read metadata tags
    pub async fn extract_metadata(file: &Path) -> Result<AudioMetadata> {
        let output = Command::new("ffprobe")
            .arg("-v")
            .arg("quiet")
            .arg("-print_format")
            .arg("json")
            .arg("-show_format")
            .arg(file.as_os_str())
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
        let probe: MetadataProbe = serde_json::from_str(&json).map_err(|e| {
            LibationError::AudioFormatDetectionFailed(format!(
                "Failed to parse metadata: {}",
                e
            ))
        })?;

        let tags = probe.format.tags.unwrap_or_default();

        Ok(AudioMetadata {
            title: tags.get("title").cloned().unwrap_or_default(),
            authors: tags
                .get("artist")
                .map(|s| vec![s.clone()])
                .unwrap_or_default(),
            narrators: tags
                .get("album_artist")
                .map(|s| vec![s.clone()])
                .unwrap_or_default(),
            publisher: tags.get("publisher").cloned(),
            publication_date: tags.get("date").cloned(),
            language: tags.get("language").cloned(),
            series: tags.get("series").map(|s| SeriesInfo {
                name: s.clone(),
                position: None,
            }),
            description: tags.get("comment").cloned(),
            genres: tags
                .get("genre")
                .map(|s| s.split(';').map(|g| g.trim().to_string()).collect())
                .unwrap_or_default(),
            runtime_minutes: None,
            asin: tags.get("asin").cloned(),
            cover_art_url: None,
        })
    }

    /// Embed cover art into audio file
    ///
    /// Based on AudioDecodable.cs cover art embedding
    pub async fn embed_cover_art(file: &Path, cover_art_path: &Path) -> Result<()> {
        // Create temporary file for output
        let temp_file = file.with_extension("tmp.m4b");

        let cmd = vec![
            "ffmpeg".to_string(),
            "-i".to_string(),
            file.to_string_lossy().to_string(),
            "-i".to_string(),
            cover_art_path.to_string_lossy().to_string(),
            "-map".to_string(),
            "0:a".to_string(),
            "-map".to_string(),
            "1:v".to_string(),
            "-codec:a".to_string(),
            "copy".to_string(),
            "-codec:v".to_string(),
            "copy".to_string(),
            "-disposition:v:0".to_string(),
            "attached_pic".to_string(),
            "-y".to_string(),
            temp_file.to_string_lossy().to_string(),
        ];

        // Execute FFmpeg
        Self::execute_ffmpeg(&cmd).await?;

        // Replace original with temp file
        fs::rename(&temp_file, file).await.map_err(|e| {
            LibationError::FileIoError(format!("{}: {} - {}", "rename".to_string(), file.to_string_lossy().to_string(), e.to_string(),
            ))
        })?;

        Ok(())
    }

    /// Extract cover art from audio file
    pub async fn extract_cover_art(file: &Path, output_path: &Path) -> Result<()> {
        let cmd = vec![
            "ffmpeg".to_string(),
            "-i".to_string(),
            file.to_string_lossy().to_string(),
            "-an".to_string(),
            "-codec:v".to_string(),
            "copy".to_string(),
            "-y".to_string(),
            output_path.to_string_lossy().to_string(),
        ];

        Self::execute_ffmpeg(&cmd).await
    }

    /// Download cover art from URL
    pub async fn download_cover_art(url: &str, output_path: &Path) -> Result<()> {
        // Use reqwest to download
        let response = reqwest::get(url).await.map_err(|e| {
            LibationError::network_error(format!("Failed to download cover art: {}", e), true)
        })?;

        if !response.status().is_success() {
            return Err(LibationError::network_error(
                format!("HTTP {} when downloading cover art", response.status()),
                true,
            ));
        }

        let bytes = response.bytes().await.map_err(|e| {
            LibationError::network_error(format!("Failed to read cover art bytes: {}", e), true)
        })?;

        fs::write(output_path, &bytes).await.map_err(|e| {
            LibationError::FileIoError(format!("{}: {} - {}", "write".to_string(), output_path.to_string_lossy().to_string(), e.to_string(),
            ))
        })
    }

    /// Execute FFmpeg command and handle errors
    async fn execute_ffmpeg(command: &[String]) -> Result<()> {
        let output = Command::new(&command[0])
            .args(&command[1..])
            .output()
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    LibationError::FfmpegNotFound
                } else {
                    LibationError::FfmpegError(format!("Failed to execute ffmpeg: {}", e))
                }
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(LibationError::FfmpegError(format!(
                "FFmpeg failed: {}",
                stderr
            )));
        }

        Ok(())
    }
}

/// Chapter editor for managing chapter markers
pub struct ChapterEditor;

impl ChapterEditor {
    /// Embed chapters into audio file
    ///
    /// Uses FFmpeg with ffmetadata file format
    pub async fn embed_chapters(file: &Path, chapters: &[Chapter]) -> Result<()> {
        // Generate ffmetadata content
        let metadata_content = Self::generate_ffmetadata(chapters);

        // Write to temporary metadata file
        let metadata_file = file.with_extension("ffmetadata.txt");
        let mut file_handle = fs::File::create(&metadata_file).await.map_err(|e| {
            LibationError::FileIoError(format!("{}: {} - {}", "create".to_string(), metadata_file.to_string_lossy().to_string(), e.to_string(),
            ))
        })?;

        file_handle
            .write_all(metadata_content.as_bytes())
            .await
            .map_err(|e| {
                LibationError::FileIoError(format!("{}: {} - {}", "write".to_string(), metadata_file.to_string_lossy().to_string(), e.to_string(),
                ))
            })?;

        file_handle.sync_all().await.map_err(|e| {
            LibationError::FileIoError(format!("{}: {} - {}", "sync".to_string(), metadata_file.to_string_lossy().to_string(), e.to_string(),
            ))
        })?;

        drop(file_handle);

        // Create temporary output file
        let temp_file = file.with_extension("tmp.m4b");

        // Build FFmpeg command
        let cmd = vec![
            "ffmpeg".to_string(),
            "-i".to_string(),
            file.to_string_lossy().to_string(),
            "-i".to_string(),
            metadata_file.to_string_lossy().to_string(),
            "-map_metadata".to_string(),
            "1".to_string(),
            "-codec".to_string(),
            "copy".to_string(),
            "-y".to_string(),
            temp_file.to_string_lossy().to_string(),
        ];

        // Execute FFmpeg
        MetadataEditor::execute_ffmpeg(&cmd).await?;

        // Clean up metadata file
        let _ = fs::remove_file(&metadata_file).await;

        // Replace original with temp file
        fs::rename(&temp_file, file).await.map_err(|e| {
            LibationError::FileIoError(format!("{}: {} - {}", "rename".to_string(), file.to_string_lossy().to_string(), e.to_string(),
            ))
        })?;

        Ok(())
    }

    /// Extract chapters from audio file
    pub async fn extract_chapters(file: &Path) -> Result<Vec<Chapter>> {
        let output = Command::new("ffprobe")
            .arg("-v")
            .arg("quiet")
            .arg("-print_format")
            .arg("json")
            .arg("-show_chapters")
            .arg(file.as_os_str())
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
                // FFprobe returns times as strings in seconds
                let start_seconds: f64 = c.start_time.parse().unwrap_or(0.0);
                let end_seconds: f64 = c.end_time.parse().unwrap_or(0.0);

                Chapter {
                    title: c.tags.and_then(|t| t.get("title").cloned()).unwrap_or_else(|| format!("Chapter {}", c.id)),
                    start_ms: (start_seconds * 1000.0) as i64,
                    end_ms: (end_seconds * 1000.0) as i64,
                }
            })
            .collect();

        Ok(chapters)
    }

    /// Generate cue sheet content
    ///
    /// Based on Cue.cs from Libation
    pub fn generate_cue_sheet(
        metadata: &AudioMetadata,
        chapters: &[Chapter],
        audio_filename: &str,
    ) -> String {
        let mut cue = String::new();

        // Header
        cue.push_str(&format!("PERFORMER \"{}\"\n", metadata.format_authors()));
        cue.push_str(&format!("TITLE \"{}\"\n", metadata.title));

        // Determine file type from extension
        let file_type = if audio_filename.ends_with(".mp3") {
            "MP3"
        } else {
            "MP4"
        };

        cue.push_str(&format!("FILE \"{}\" {}\n", audio_filename, file_type));

        // Chapters
        for (idx, chapter) in chapters.iter().enumerate() {
            let track_num = idx + 1;
            cue.push_str(&format!("  TRACK {:02} AUDIO\n", track_num));
            cue.push_str(&format!("    TITLE \"{}\"\n", chapter.title));
            cue.push_str(&format!(
                "    INDEX 01 {}\n",
                Chapter::format_cue_timestamp(chapter.start_ms)
            ));
        }

        cue
    }

    /// Save cue sheet to file
    pub async fn save_cue_sheet(
        file: &Path,
        metadata: &AudioMetadata,
        chapters: &[Chapter],
    ) -> Result<()> {
        let audio_filename = file
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| {
                LibationError::InvalidPath(format!(
                    "Invalid audio file path: {}",
                    file.display()
                ))
            })?;

        let cue_content = Self::generate_cue_sheet(metadata, chapters, audio_filename);
        let cue_path = file.with_extension("cue");

        fs::write(&cue_path, cue_content).await.map_err(|e| {
            LibationError::FileIoError(format!("{}: {} - {}", "write".to_string(), cue_path.to_string_lossy().to_string(), e.to_string(),
            ))
        })
    }

    /// Generate FFmetadata format content
    ///
    /// Used by FFmpeg for chapter embedding
    fn generate_ffmetadata(chapters: &[Chapter]) -> String {
        let mut content = String::from(";FFMETADATA1\n");

        for chapter in chapters {
            content.push_str("\n[CHAPTER]\n");
            content.push_str("TIMEBASE=1/1000\n");
            content.push_str(&format!("START={}\n", chapter.start_ms));
            content.push_str(&format!("END={}\n", chapter.end_ms));
            content.push_str(&format!("title={}\n", chapter.title));
        }

        content
    }
}

/// FFprobe metadata output structures
#[derive(Debug, Deserialize)]
struct MetadataProbe {
    format: MetadataFormat,
}

#[derive(Debug, Deserialize)]
struct MetadataFormat {
    tags: Option<std::collections::HashMap<String, String>>,
}

/// FFprobe chapter output
#[derive(Debug, Deserialize)]
struct ChapterProbe {
    chapters: Option<Vec<ChapterProbeEntry>>,
}

#[derive(Debug, Deserialize)]
struct ChapterProbeEntry {
    id: i64,
    start_time: String,
    end_time: String,
    tags: Option<std::collections::HashMap<String, String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chapter_duration() {
        let chapter = Chapter {
            title: "Chapter 1".to_string(),
            start_ms: 0,
            end_ms: 300000, // 5 minutes
        };
        assert_eq!(chapter.duration_ms(), 300000);
    }

    #[test]
    fn test_format_cue_timestamp() {
        // 0 seconds
        assert_eq!(Chapter::format_cue_timestamp(0), "00:00:00");
        // 30.5 seconds
        assert_eq!(Chapter::format_cue_timestamp(30500), "00:30:37");
        // 5 minutes
        assert_eq!(Chapter::format_cue_timestamp(300000), "05:00:00");
        // 1 hour
        assert_eq!(Chapter::format_cue_timestamp(3600000), "60:00:00");
    }

    #[test]
    fn test_format_ffmpeg_timestamp() {
        assert_eq!(Chapter::format_ffmpeg_timestamp(0), "00:00:00.000");
        assert_eq!(
            Chapter::format_ffmpeg_timestamp(30500),
            "00:00:30.500"
        );
        assert_eq!(
            Chapter::format_ffmpeg_timestamp(3665123),
            "01:01:05.123"
        );
    }

    #[test]
    fn test_metadata_format_authors() {
        let metadata = AudioMetadata {
            title: "Test Book".to_string(),
            authors: vec!["Author 1".to_string(), "Author 2".to_string()],
            narrators: vec![],
            publisher: None,
            publication_date: None,
            language: None,
            series: None,
            description: None,
            genres: vec![],
            runtime_minutes: None,
            asin: None,
            cover_art_url: None,
        };
        assert_eq!(metadata.format_authors(), "Author 1, Author 2");
    }

    #[test]
    fn test_metadata_format_series() {
        let metadata = AudioMetadata {
            title: "Test Book".to_string(),
            authors: vec![],
            narrators: vec![],
            publisher: None,
            publication_date: None,
            language: None,
            series: Some(SeriesInfo {
                name: "Test Series".to_string(),
                position: Some("1".to_string()),
            }),
            description: None,
            genres: vec![],
            runtime_minutes: None,
            asin: None,
            cover_art_url: None,
        };
        assert_eq!(metadata.format_series(), Some("Test Series #1".to_string()));
    }

    #[test]
    fn test_generate_ffmetadata() {
        let chapters = vec![
            Chapter {
                title: "Chapter 1".to_string(),
                start_ms: 0,
                end_ms: 300000,
            },
            Chapter {
                title: "Chapter 2".to_string(),
                start_ms: 300000,
                end_ms: 600000,
            },
        ];

        let metadata = ChapterEditor::generate_ffmetadata(&chapters);
        assert!(metadata.contains(";FFMETADATA1"));
        assert!(metadata.contains("[CHAPTER]"));
        assert!(metadata.contains("TIMEBASE=1/1000"));
        assert!(metadata.contains("START=0"));
        assert!(metadata.contains("END=300000"));
        assert!(metadata.contains("title=Chapter 1"));
        assert!(metadata.contains("START=300000"));
        assert!(metadata.contains("END=600000"));
        assert!(metadata.contains("title=Chapter 2"));
    }

    #[test]
    fn test_generate_cue_sheet() {
        let metadata = AudioMetadata {
            title: "Test Audiobook".to_string(),
            authors: vec!["John Doe".to_string()],
            narrators: vec![],
            publisher: None,
            publication_date: None,
            language: None,
            series: None,
            description: None,
            genres: vec![],
            runtime_minutes: None,
            asin: None,
            cover_art_url: None,
        };

        let chapters = vec![
            Chapter {
                title: "Prologue".to_string(),
                start_ms: 0,
                end_ms: 180000,
            },
            Chapter {
                title: "Chapter 1".to_string(),
                start_ms: 180000,
                end_ms: 600000,
            },
        ];

        let cue = ChapterEditor::generate_cue_sheet(&metadata, &chapters, "audiobook.m4b");
        assert!(cue.contains("PERFORMER \"John Doe\""));
        assert!(cue.contains("TITLE \"Test Audiobook\""));
        assert!(cue.contains("FILE \"audiobook.m4b\" MP4"));
        assert!(cue.contains("TRACK 01 AUDIO"));
        assert!(cue.contains("TITLE \"Prologue\""));
        assert!(cue.contains("INDEX 01 00:00:00"));
        assert!(cue.contains("TRACK 02 AUDIO"));
        assert!(cue.contains("TITLE \"Chapter 1\""));
    }
}
