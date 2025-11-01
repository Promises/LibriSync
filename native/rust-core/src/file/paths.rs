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


//! Path generation and naming templates
//!
//! # Reference C# Sources
//! - `FileManager/NamingTemplate/NamingTemplate.cs` - Template engine
//! - `FileManager/ReplacementCharacters.cs` - Character replacement
//! - `FileManager/FileUtility.cs` - Path sanitization
//! - `LibationFileManager/Configuration.LibationFiles.cs` - Default paths
//!
//! # Template System
//! - Templates use placeholders: `{title}`, `{author}`, etc.
//! - Sanitize for filesystem compatibility
//! - Handle path length limits
//! - Avoid filename collisions

use crate::audio::metadata::AudioMetadata;
use crate::error::{LibationError, Result};
use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// Platform-specific path limits (in bytes for UTF-8)
#[cfg(target_os = "windows")]
const MAX_PATH_LENGTH: usize = 260;
#[cfg(not(target_os = "windows"))]
const MAX_PATH_LENGTH: usize = 4096;

#[cfg(target_os = "windows")]
const MAX_COMPONENT_LENGTH: usize = 255;
#[cfg(target_os = "macos")]
const MAX_COMPONENT_LENGTH: usize = 255; // 255 bytes in UTF-8
#[cfg(target_os = "linux")]
const MAX_COMPONENT_LENGTH: usize = 255; // 255 bytes
#[cfg(target_os = "android")]
const MAX_COMPONENT_LENGTH: usize = 255;
#[cfg(target_os = "ios")]
const MAX_COMPONENT_LENGTH: usize = 255;
#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "linux",
    target_os = "android",
    target_os = "ios"
)))]
const MAX_COMPONENT_LENGTH: usize = 255;

/// Path template for generating filenames from book metadata
///
/// # Reference: `FileManager/NamingTemplate/NamingTemplate.cs`
#[derive(Debug, Clone)]
pub struct PathTemplate {
    template: String,
}

impl PathTemplate {
    pub fn new(template: String) -> Self {
        Self { template }
    }

    /// Flat file: `{title}.m4b`
    /// Example: "All These Worlds.m4b"
    pub fn flat_file() -> Self {
        Self::new("{title}".to_string())
    }

    /// Author/Book folder: `{author}/{title}/{title}.m4b`
    /// Example: "Dennis E. Taylor/All These Worlds/All These Worlds.m4b"
    pub fn author_book_folder() -> Self {
        Self::new("{author}/{title}/{title}".to_string())
    }

    /// Author/Series+Book: `{author}/{series} {series_seq} - {title_no_series}/{series} {series_seq} - {title_no_series}.m4b`
    /// Example: "Dennis E. Taylor/Bobiverse 3 - All These Worlds/Bobiverse 3 - All These Worlds.m4b"
    /// Falls back to author/title/title if no series
    pub fn author_series_book() -> Self {
        Self::new("{author}/{series_folder}/{series_file}".to_string())
    }

    /// Legacy: Default template: `{author}/{title}`
    pub fn default_audiobook() -> Self {
        Self::new("{author}/{title}".to_string())
    }

    /// Legacy: Series template: `{author}/{series}/{title}`
    pub fn default_series() -> Self {
        Self::new("{author}/{series}/{title}".to_string())
    }

    /// Legacy: With series number: `{author}/{series} #{series_seq} - {title}`
    pub fn default_series_numbered() -> Self {
        Self::new("{author}/{series} #{series_seq} - {title}".to_string())
    }

    /// Legacy: Simple template: `{title} - {author}`
    pub fn simple() -> Self {
        Self::new("{title} - {author}".to_string())
    }

    /// Render template with metadata
    ///
    /// # Reference: `FileManager/NamingTemplate/NamingTemplate.cs` Evaluate()
    pub fn render(&self, metadata: &AudioMetadata) -> Result<String> {
        let tags = Self::extract_tags(metadata);
        let mut result = self.template.clone();

        // Replace all tags in template
        for (tag, value) in tags.iter() {
            let placeholder = format!("{{{}}}", tag);
            result = result.replace(&placeholder, value);
        }

        // Remove any remaining unreplaced tags
        let tag_regex = Regex::new(r"\{[a-z_]+\}").unwrap();
        result = tag_regex.replace_all(&result, "").to_string();

        // Clean up multiple slashes and whitespace
        result = Self::clean_path_string(&result);

        Ok(result)
    }

    /// Extract tags from metadata
    ///
    /// # Reference: `FileManager/NamingTemplate/PropertyTagCollection.cs`
    fn extract_tags(metadata: &AudioMetadata) -> HashMap<String, String> {
        let mut tags = HashMap::new();

        // Title
        tags.insert("title".to_string(), metadata.title.clone());

        // Author(s)
        if !metadata.authors.is_empty() {
            tags.insert("author".to_string(), metadata.authors[0].clone());
            tags.insert("authors".to_string(), metadata.format_authors());
        } else {
            tags.insert("author".to_string(), "Unknown Author".to_string());
            tags.insert("authors".to_string(), "Unknown Author".to_string());
        }

        // Narrator(s)
        if !metadata.narrators.is_empty() {
            tags.insert("narrator".to_string(), metadata.narrators[0].clone());
            tags.insert("narrators".to_string(), metadata.format_narrators());
        }

        // Series
        let mut title_no_series = metadata.title.clone();
        if let Some(ref series) = metadata.series {
            tags.insert("series".to_string(), series.name.clone());

            let series_seq = series.position.as_ref()
                .map(|p| p.clone())
                .unwrap_or_default();

            tags.insert("series_seq".to_string(), series_seq.clone());

            // Strip series name from title for {title_no_series} tag
            // Remove patterns like "Bobiverse 3 - " or "Cradle: " or "(Cradle Book 3)"
            let series_lower = series.name.to_lowercase();
            let title_lower = title_no_series.to_lowercase();

            // Try to remove series name from beginning
            if title_lower.starts_with(&series_lower) {
                let after_series = &title_no_series[series.name.len()..];
                // Remove common separators: " - ", ": ", " ", "#X - "
                title_no_series = after_series
                    .trim_start_matches(':')
                    .trim_start_matches('-')
                    .trim_start()
                    .to_string();
            }

            // Try to remove series name from parentheses
            let paren_pattern = format!("({})", series.name);
            if title_no_series.contains(&paren_pattern) {
                title_no_series = title_no_series.replace(&paren_pattern, "").trim().to_string();
            }

            // Build series_folder and series_file tags for Author/Series+Book pattern
            // Format: "SeriesName Seq - Title"
            let series_folder = if !series_seq.is_empty() {
                format!("{} {} - {}", series.name, series_seq, metadata.title)
            } else {
                format!("{} - {}", series.name, metadata.title)
            };

            tags.insert("series_folder".to_string(), series_folder.clone());
            tags.insert("series_file".to_string(), series_folder);
        } else {
            // No series - fall back to title/title pattern
            tags.insert("series_folder".to_string(), metadata.title.clone());
            tags.insert("series_file".to_string(), metadata.title.clone());
        }
        tags.insert("title_no_series".to_string(), title_no_series);

        // Year
        if let Some(ref date) = metadata.publication_date {
            // Extract year from date (format: YYYY-MM-DD or just YYYY)
            if let Some(year) = date.split('-').next() {
                tags.insert("year".to_string(), year.to_string());
            }
        }

        // ASIN
        if let Some(ref asin) = metadata.asin {
            tags.insert("asin".to_string(), asin.clone());
        }

        // Genre (first one)
        if !metadata.genres.is_empty() {
            tags.insert("genre".to_string(), metadata.genres[0].clone());
        }

        tags
    }

    /// Clean up path string
    fn clean_path_string(path: &str) -> String {
        // Replace multiple slashes with single slash
        let slash_regex = Regex::new(r"/+").unwrap();
        let result = slash_regex.replace_all(path, "/");

        // Trim whitespace around slashes and at ends
        let parts: Vec<&str> = result.split('/').map(|s| s.trim()).collect();
        let result = parts
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<&str>>()
            .join("/");

        result
    }
}

/// Path builder for constructing full file paths
///
/// # Reference: Multiple C# sources combined
#[derive(Debug)]
pub struct PathBuilder {
    base_directory: PathBuf,
    template: PathTemplate,
}

impl PathBuilder {
    pub fn new(base_directory: PathBuf, template: PathTemplate) -> Self {
        Self {
            base_directory,
            template,
        }
    }

    /// Build full path from metadata
    ///
    /// # Reference: `LibationFileManager/Configuration.cs` and `FileManager/FileUtility.cs`
    pub fn build_path(&self, metadata: &AudioMetadata, extension: &str) -> Result<PathBuf> {
        // Render template
        let relative_path = self.template.render(metadata)?;

        // Split into directory and filename
        let parts: Vec<&str> = relative_path.split('/').collect();
        let (dir_parts, filename) = if parts.len() > 1 {
            (&parts[..parts.len() - 1], parts[parts.len() - 1])
        } else {
            (&[][..], parts[0])
        };

        // Sanitize each directory component
        let mut sanitized_dirs = Vec::new();
        for part in dir_parts {
            let sanitized = sanitize_path_component(part);
            let truncated = truncate_component(&sanitized, MAX_COMPONENT_LENGTH);
            sanitized_dirs.push(truncated);
        }

        // Sanitize filename
        let sanitized_filename = sanitize_filename(filename);
        let ext = if extension.starts_with('.') {
            extension.to_string()
        } else {
            format!(".{}", extension)
        };

        // Reserve space for extension and potential collision suffix " (999)"
        let max_filename_len = MAX_COMPONENT_LENGTH - ext.len() - 6;
        let truncated_filename = truncate_component(&sanitized_filename, max_filename_len);

        // Build full path
        let mut path = self.base_directory.clone();
        for dir in sanitized_dirs {
            path.push(dir);
        }

        let final_filename = format!("{}{}", truncated_filename, ext);
        path.push(&final_filename);

        // Ensure total path doesn't exceed limits
        let path_str = path.to_string_lossy();
        if path_str.as_bytes().len() > MAX_PATH_LENGTH {
            return Err(LibationError::InvalidPath(format!(
                "Path too long ({} bytes): {}",
                path_str.as_bytes().len(),
                path_str
            )));
        }

        Ok(path)
    }

    /// Build path for cover art
    pub fn build_cover_path(&self, metadata: &AudioMetadata) -> Result<PathBuf> {
        self.build_path(metadata, "jpg")
    }

    /// Build path for cue sheet
    pub fn build_cue_path(&self, audio_path: &Path) -> PathBuf {
        audio_path.with_extension("cue")
    }
}

/// Get default library path for the platform
///
/// # Reference: `LibationFileManager/Configuration.LibationFiles.cs`
pub fn get_default_library_path() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            let mut path = PathBuf::from(home);
            path.push("Library");
            path.push("Application Support");
            path.push("LibriSync");
            path.push("Library");
            return path;
        }
        return PathBuf::from("./library");
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            let mut path = PathBuf::from(home);
            path.push(".local");
            path.push("share");
            path.push("librisync");
            path.push("library");
            return path;
        }
        return PathBuf::from("./library");
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let mut path = PathBuf::from(appdata);
            path.push("LibriSync");
            path.push("Library");
            return path;
        }
        return PathBuf::from("./library");
    }

    #[cfg(target_os = "android")]
    {
        // Android: Use app-specific directory
        // This will need to be passed from Java/Kotlin layer
        PathBuf::from("/data/data/com.librisync/files/library")
    }

    #[cfg(target_os = "ios")]
    {
        // iOS: Use documents directory
        // This will need to be determined at runtime
        if let Some(home) = std::env::var_os("HOME") {
            let mut path = PathBuf::from(home);
            path.push("Documents");
            path.push("library");
            return path;
        }
        PathBuf::from("./library")
    }

    #[cfg(not(any(
        target_os = "macos",
        target_os = "linux",
        target_os = "windows",
        target_os = "android",
        target_os = "ios"
    )))]
    {
        PathBuf::from("./library")
    }
}

/// Sanitize filename (removes/replaces invalid characters for filenames)
///
/// # Reference: `FileManager/ReplacementCharacters.cs` ReplaceFilenameChars()
pub fn sanitize_filename(name: &str) -> String {
    // First, strip commas entirely
    let without_commas = name.replace(',', "");

    let mut result = String::with_capacity(without_commas.len());

    for (i, c) in without_commas.chars().enumerate() {
        let is_last = i == without_commas.len() - 1;
        let next_char = if !is_last {
            without_commas.chars().nth(i + 1)
        } else {
            None
        };
        let prev_char = if i > 0 { without_commas.chars().nth(i - 1) } else { None };

        result.push(replace_char(c, prev_char, next_char, true));
    }

    // Trim leading/trailing whitespace and dots
    result = result.trim().trim_end_matches('.').to_string();

    // Handle reserved names on Windows
    if cfg!(target_os = "windows") {
        result = handle_windows_reserved_names(&result);
    }

    // Ensure not empty
    if result.is_empty() {
        result = "file".to_string();
    }

    result
}

/// Sanitize path component (directory name)
///
/// # Reference: `FileManager/FileUtility.cs` GetSafePath()
pub fn sanitize_path_component(name: &str) -> String {
    // First, strip commas entirely
    let without_commas = name.replace(',', "");

    let mut result = String::with_capacity(without_commas.len());

    for (i, c) in without_commas.chars().enumerate() {
        let is_last = i == without_commas.len() - 1;
        let next_char = if !is_last {
            without_commas.chars().nth(i + 1)
        } else {
            None
        };
        let prev_char = if i > 0 { without_commas.chars().nth(i - 1) } else { None };

        // For path components, we don't replace slashes
        if c == '/' || c == '\\' {
            continue;
        }

        result.push(replace_char(c, prev_char, next_char, false));
    }

    // Trim leading/trailing whitespace and dots
    result = result.trim().trim_end_matches('.').to_string();

    // Handle reserved names on Windows
    if cfg!(target_os = "windows") {
        result = handle_windows_reserved_names(&result);
    }

    // Ensure not empty
    if result.is_empty() {
        result = "folder".to_string();
    }

    result
}

/// Replace invalid character with safe alternative
///
/// # Reference: `FileManager/ReplacementCharacters.cs` GetFilenameCharReplacement()
fn replace_char(
    c: char,
    prev_char: Option<char>,
    next_char: Option<char>,
    is_filename: bool,
) -> char {
    // Smart quote handling
    if c == '"' {
        // Opening quote: at start or after non-alphanumeric
        if prev_char.is_none()
            || prev_char.map_or(false, |p| !p.is_alphanumeric() && next_char.map_or(false, |n| n.is_alphanumeric()))
        {
            return '"'; // U+201C left double quotation mark
        }
        // Closing quote: at end or before non-alphanumeric
        else if next_char.is_none()
            || next_char.map_or(false, |n| !n.is_alphanumeric() && prev_char.map_or(false, |p| p.is_alphanumeric()))
        {
            return '"'; // U+201D right double quotation mark
        }
        // Other quote
        return '＂'; // U+FF02 fullwidth quotation mark
    }

    // Platform-specific invalid characters
    match c {
        '<' => '＜', // U+FF1C fullwidth less-than sign
        '>' => '＞', // U+FF1E fullwidth greater-than sign
        ':' => '_',  // Colon is problematic on many systems
        '|' => '⏐', // U+23D0 vertical line extension
        '?' => '？', // U+FF1F fullwidth question mark
        '*' => '✱', // U+2731 heavy asterisk
        '/' if is_filename => '∕', // U+2215 division slash (only for filenames)
        '\\' if is_filename => '_', // Backslash in filename
        '\0' => '_', // Null byte
        c if c.is_control() => '_', // Control characters
        c => c,      // Valid character
    }
}

/// Handle Windows reserved filenames
///
/// # Reference: `FileManager/ReplacementCharacters.cs` and Windows filesystem docs
fn handle_windows_reserved_names(name: &str) -> String {
    let upper = name.to_uppercase();
    let reserved = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
        "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];

    for reserved_name in &reserved {
        if upper == *reserved_name || upper.starts_with(&format!("{}.", reserved_name)) {
            return format!("_{}", name);
        }
    }

    name.to_string()
}

/// Truncate path component to fit within byte limit
///
/// # Reference: `FileManager/FileUtility.cs` TruncateFilename()
pub fn truncate_component(text: &str, max_bytes: usize) -> String {
    let bytes = text.as_bytes();
    if bytes.len() <= max_bytes {
        return text.to_string();
    }

    // Find valid UTF-8 boundary
    let mut index = max_bytes;
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }

    // Leave room for ellipsis if we truncated significantly
    if index < max_bytes - 3 {
        format!("{}...", &text[..index.saturating_sub(3)])
    } else {
        text[..index].to_string()
    }
}

/// Avoid filename collision by appending (1), (2), etc.
///
/// # Reference: `FileManager/FileUtility.cs` GetValidFilename()
pub fn avoid_collision(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }

    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("");

    let mut counter = 1;
    loop {
        let new_name = if extension.is_empty() {
            format!("{} ({})", stem, counter)
        } else {
            format!("{} ({}).{}", stem, counter, extension)
        };

        let new_path = parent.join(new_name);
        if !new_path.exists() {
            return new_path;
        }

        counter += 1;
        if counter > 9999 {
            // Safety limit
            return new_path;
        }
    }
}

/// Naming pattern for audiobook files
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamingPattern {
    /// Flat file: `{title}.m4b`
    /// Example: "All These Worlds.m4b"
    FlatFile,

    /// Author/Book folder: `{author}/{title}/{title}.m4b`
    /// Example: "Dennis E. Taylor/All These Worlds/All These Worlds.m4b"
    AuthorBookFolder,

    /// Author/Series+Book: `{author}/{series} {series_seq} - {title_no_series}/{series} {series_seq} - {title_no_series}.m4b`
    /// Example: "Dennis E. Taylor/Bobiverse 3 - All These Worlds/Bobiverse 3 - All These Worlds.m4b"
    /// Falls back to author/title/title if no series
    AuthorSeriesBook,
}

impl NamingPattern {
    pub fn to_template(&self) -> PathTemplate {
        match self {
            NamingPattern::FlatFile => PathTemplate::flat_file(),
            NamingPattern::AuthorBookFolder => PathTemplate::author_book_folder(),
            NamingPattern::AuthorSeriesBook => PathTemplate::author_series_book(),
        }
    }

    pub fn from_string(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "flat" | "flat_file" => Some(NamingPattern::FlatFile),
            "author_book" | "author_book_folder" => Some(NamingPattern::AuthorBookFolder),
            "author_series" | "author_series_book" => Some(NamingPattern::AuthorSeriesBook),
            _ => None,
        }
    }
}

/// Build file path using specified naming pattern
///
/// Returns the relative path (no base directory) ready to append to output directory.
/// Example: "Dennis E. Taylor/Bobiverse 3 - All These Worlds/Bobiverse 3 - All These Worlds.m4b"
pub fn build_file_path(
    metadata: &AudioMetadata,
    pattern: NamingPattern,
    extension: &str,
) -> Result<String> {
    let template = pattern.to_template();
    let rendered = template.render(metadata)?;

    // Split into path components and sanitize each
    let parts: Vec<&str> = rendered.split('/').collect();
    let mut sanitized_parts = Vec::new();

    for (i, part) in parts.iter().enumerate() {
        let is_last = i == parts.len() - 1;

        if is_last {
            // Last part is the filename
            let sanitized = sanitize_filename(part);
            sanitized_parts.push(sanitized);
        } else {
            // Directory component
            let sanitized = sanitize_path_component(part);
            if !sanitized.is_empty() {
                sanitized_parts.push(sanitized);
            }
        }
    }

    let result = sanitized_parts.join("/");

    // Add extension if needed
    let with_ext = if extension.is_empty() || result.ends_with(extension) {
        result
    } else {
        format!("{}.{}", result, extension.trim_start_matches('.'))
    };

    Ok(with_ext)
}

/// Get a safe, unique filename
///
/// # Reference: `FileManager/FileUtility.cs` GetValidFilename()
pub fn get_safe_filename(
    base_path: &Path,
    metadata: &AudioMetadata,
    template: &PathTemplate,
    extension: &str,
) -> Result<PathBuf> {
    let builder = PathBuilder::new(base_path.to_path_buf(), template.clone());
    let path = builder.build_path(metadata, extension)?;

    // Avoid collision
    Ok(avoid_collision(&path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::metadata::SeriesInfo;

    fn test_metadata() -> AudioMetadata {
        AudioMetadata {
            title: "Test Book".to_string(),
            authors: vec!["John Doe".to_string()],
            narrators: vec!["Jane Smith".to_string()],
            publisher: Some("Test Publisher".to_string()),
            publication_date: Some("2023".to_string()),
            language: Some("en".to_string()),
            series: Some(SeriesInfo {
                name: "Test Series".to_string(),
                position: Some("1".to_string()),
            }),
            description: None,
            genres: vec!["Fiction".to_string()],
            runtime_minutes: Some(600),
            asin: Some("B001TEST".to_string()),
            cover_art_url: None,
        }
    }

    #[test]
    fn test_render_simple_template() {
        let template = PathTemplate::new("{title} - {author}".to_string());
        let metadata = test_metadata();
        let result = template.render(&metadata).unwrap();
        assert_eq!(result, "Test Book - John Doe");
    }

    #[test]
    fn test_render_path_template() {
        let template = PathTemplate::new("{author}/{title}".to_string());
        let metadata = test_metadata();
        let result = template.render(&metadata).unwrap();
        assert_eq!(result, "John Doe/Test Book");
    }

    #[test]
    fn test_render_series_template() {
        let template = PathTemplate::new("{author}/{series} #{series_seq} - {title}".to_string());
        let metadata = test_metadata();
        let result = template.render(&metadata).unwrap();
        assert_eq!(result, "John Doe/Test Series #1 - Test Book");
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("test<>file"), "test＜＞file");
        assert_eq!(sanitize_filename("test:file"), "test_file");
        assert_eq!(sanitize_filename("test|file"), "test⏐file");
        assert_eq!(sanitize_filename("test?file"), "test？file");
        assert_eq!(sanitize_filename("test*file"), "test✱file");
        assert_eq!(sanitize_filename("test/file"), "test∕file");
    }

    #[test]
    fn test_sanitize_path_component() {
        // Path components should not contain slashes at all
        assert_eq!(sanitize_path_component("test/folder"), "testfolder");
        assert_eq!(sanitize_path_component("test:folder"), "test_folder");
    }

    #[test]
    fn test_truncate_component() {
        let long_text = "a".repeat(300);
        let truncated = truncate_component(&long_text, 255);
        assert!(truncated.as_bytes().len() <= 255);
    }

    #[test]
    fn test_trim_whitespace_and_dots() {
        assert_eq!(sanitize_filename("  test  "), "test");
        assert_eq!(sanitize_filename("test..."), "test");
        assert_eq!(sanitize_filename("  test...  "), "test");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_windows_reserved_names() {
        assert_eq!(sanitize_filename("CON"), "_CON");
        assert_eq!(sanitize_filename("PRN"), "_PRN");
        assert_eq!(sanitize_filename("AUX"), "_AUX");
        assert_eq!(sanitize_filename("NUL"), "_NUL");
        assert_eq!(sanitize_filename("COM1"), "_COM1");
        assert_eq!(sanitize_filename("CON.txt"), "_CON.txt");
    }
}
