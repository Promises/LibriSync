//! Chapter lists: the FFMETADATA chapter file, the concat-demuxer list, and
//! `NN - Title.mp3` naming.
//!
//! Port of `writeJoinFiles`, `escapeFfmetadata` and the chapter naming in
//! `encodeChaptersToMp3` (`modules/expo-rust-bridge/android/.../DownloadOrchestrator.kt`).

use std::path::Path;

use crate::providers::PlanChapter;

use super::fsutil::sanitize_file_name;

/// One chapter, in milliseconds from the start of the book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Chapter {
    pub title: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

impl Chapter {
    pub(crate) fn duration_ms(&self) -> u64 {
        self.end_ms.saturating_sub(self.start_ms)
    }
}

/// The plan's chapters, minus empty ones (`end <= start`), with blank titles filled in.
pub(crate) fn from_plan(chapters: &[PlanChapter]) -> Vec<Chapter> {
    normalise(
        chapters
            .iter()
            .map(|c| Chapter {
                title: c.title.clone(),
                start_ms: c.start_ms,
                end_ms: c.end_ms,
            })
            .collect(),
    )
}

/// Drop chapters with `end <= start` and give untitled ones `Chapter n`.
pub(crate) fn normalise(chapters: Vec<Chapter>) -> Vec<Chapter> {
    chapters
        .into_iter()
        .filter(|c| c.end_ms > c.start_ms)
        .enumerate()
        .map(|(i, mut c)| {
            if c.title.trim().is_empty() {
                c.title = format!("Chapter {}", i + 1);
            }
            c
        })
        .collect()
}

/// Escape a value for an FFMETADATA file: `=`, `;`, `#`, `\` and newline are prefixed
/// with a backslash. Carriage returns and NULs are dropped — they have no escaped form.
pub(crate) fn escape_ffmetadata(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '=' | ';' | '#' | '\\' | '\n' => {
                out.push('\\');
                out.push(c);
            }
            '\r' | '\0' => {}
            c => out.push(c),
        }
    }
    out
}

/// An `;FFMETADATA1` file holding only chapters (absolute ms, `TIMEBASE=1/1000`).
pub(crate) fn ffmetadata(chapters: &[Chapter]) -> String {
    let mut out = String::from(";FFMETADATA1\n");
    for chapter in chapters {
        out.push_str("[CHAPTER]\nTIMEBASE=1/1000\n");
        out.push_str(&format!("START={}\nEND={}\n", chapter.start_ms, chapter.end_ms));
        out.push_str(&format!("title={}\n", escape_ffmetadata(&chapter.title)));
    }
    out
}

/// A concat-demuxer list: one `file '<path>'` line per input, `'` escaped as `'\''`.
pub(crate) fn concat_list(paths: &[&Path]) -> String {
    let mut out = String::from("ffconcat version 1.0\n");
    for path in paths {
        let escaped = path.to_string_lossy().replace('\'', "'\\''");
        out.push_str(&format!("file '{escaped}'\n"));
    }
    out
}

/// File name for chapter `n` (1-based) of `total`: `NN - Title.mp3`. The number is
/// zero-padded to at least two digits, and to the width of `total` beyond 99, so the
/// files sort in playback order.
pub(crate) fn chapter_file_name(n: usize, total: usize, title: &str) -> String {
    let width = total.to_string().len().max(2);
    let fallback = format!("Chapter {n}");
    format!(
        "{n:0width$} - {}.mp3",
        sanitize_file_name(title, &fallback),
        width = width
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(title: &str, start_ms: u64, end_ms: u64) -> Chapter {
        Chapter { title: title.into(), start_ms, end_ms }
    }

    #[test]
    fn ffmetadata_escapes_special_characters() {
        assert_eq!(escape_ffmetadata("a=b;c#d\\e"), "a\\=b\\;c\\#d\\\\e");
        assert_eq!(escape_ffmetadata("line1\nline2"), "line1\\\nline2");
        assert_eq!(escape_ffmetadata("crlf\r\nend"), "crlf\\\nend");
        assert_eq!(escape_ffmetadata("plain title"), "plain title");
    }

    #[test]
    fn ffmetadata_lists_chapters_in_milliseconds() {
        let text = ffmetadata(&[ch("One", 0, 1500), ch("Two = 2", 1500, 3000)]);
        assert_eq!(
            text,
            ";FFMETADATA1\n\
             [CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=1500\ntitle=One\n\
             [CHAPTER]\nTIMEBASE=1/1000\nSTART=1500\nEND=3000\ntitle=Two \\= 2\n"
        );
    }

    #[test]
    fn concat_list_quotes_paths() {
        let list = concat_list(&[Path::new("/w/001.m4a"), Path::new("/w/it's.m4a")]);
        assert_eq!(
            list,
            "ffconcat version 1.0\nfile '/w/001.m4a'\nfile '/w/it'\\''s.m4a'\n"
        );
    }

    #[test]
    fn chapter_names_are_padded_and_sanitised() {
        assert_eq!(chapter_file_name(1, 12, "Opening Credits"), "01 - Opening Credits.mp3");
        assert_eq!(chapter_file_name(7, 9, "Part 2: Home/Away"), "07 - Part 2 Home Away.mp3");
        assert_eq!(chapter_file_name(3, 150, "Three"), "003 - Three.mp3");
        assert_eq!(chapter_file_name(150, 150, "Last"), "150 - Last.mp3");
        assert_eq!(chapter_file_name(4, 10, "  ?? "), "04 - Chapter 4.mp3");
    }

    #[test]
    fn normalise_drops_empty_chapters_and_names_untitled_ones() {
        let out = normalise(vec![ch("", 0, 1000), ch("Zero", 1000, 1000), ch("B", 1000, 2000)]);
        assert_eq!(out, vec![ch("Chapter 1", 0, 1000), ch("B", 1000, 2000)]);
    }
}
