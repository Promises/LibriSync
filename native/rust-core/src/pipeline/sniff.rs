//! Reject a "download" that is not media at all.
//!
//! A store that refuses a download does not always say so with an HTTP error: Audible's
//! legacy AAX service answered a title the customer wasn't entitled to with a short text
//! body and `200 OK`. Left alone, that file goes on to decrypt or validation, fails there
//! as "Invalid data found when processing input", and looks like a corrupt download —
//! which a caller retries, spending another licence each time. Looking at the first bytes
//! turns it into what it is: the store sent an error page, not a book.
//!
//! Deliberately conservative: a file is rejected only when it carries none of the media
//! signatures below **and** reads as text. Anything binary that isn't recognised still
//! goes on to ffmpeg, which is the real judge.

use std::path::Path;

use tokio::io::AsyncReadExt;

use super::{FailureKind, PipelineError};

/// How much of the file is looked at.
const HEAD_BYTES: usize = 4096;
/// Longest excerpt of the body quoted in the error.
const EXCERPT_CHARS: usize = 200;

/// Media containers the pipeline can be handed: MP4 family (M4A/M4B/AAX/AAXC — an
/// `ftyp` box first, or another top-level box), MP3 (ID3 tag or MPEG frame sync, which
/// also covers ADTS AAC), ZIP, FLAC, Ogg, WAV.
fn looks_like_media(head: &[u8]) -> bool {
    const MP4_BOXES: [&[u8; 4]; 7] = [b"ftyp", b"moov", b"mdat", b"free", b"skip", b"wide", b"pnot"];
    if head.len() >= 8 && MP4_BOXES.iter().any(|b| &head[4..8] == *b) {
        return true;
    }
    if head.starts_with(b"ID3")
        || head.starts_with(b"PK\x03\x04")
        || head.starts_with(b"fLaC")
        || head.starts_with(b"OggS")
        || head.starts_with(b"RIFF")
    {
        return true;
    }
    // MPEG audio frame sync: 11 set bits.
    head.len() >= 2 && head[0] == 0xFF && head[1] & 0xE0 == 0xE0
}

/// Text rather than binary: an HTML/XML/JSON opening, or almost entirely printable.
fn looks_like_text(head: &[u8]) -> bool {
    if head.is_empty() {
        return true;
    }
    let body = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    let first = body.iter().find(|b| !b.is_ascii_whitespace());
    if matches!(first, Some(b'<' | b'{' | b'[')) {
        return true;
    }
    let printable = body
        .iter()
        .filter(|&&b| b.is_ascii_graphic() || b.is_ascii_whitespace() || b >= 0x80)
        .count();
    // A NUL never occurs in text and almost always early in a binary file.
    !body.contains(&0) && printable * 100 >= body.len() * 95
}

/// A short, log-safe quote of what the server sent: markup stripped, whitespace
/// collapsed, URLs and long token-like words replaced, at most [`EXCERPT_CHARS`].
pub(crate) fn excerpt(head: &[u8]) -> String {
    let text = String::from_utf8_lossy(head);
    let mut plain = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => {
                in_tag = true;
                plain.push(' ');
            }
            '>' if in_tag => in_tag = false,
            _ if in_tag => {}
            c if c.is_control() => plain.push(' '),
            c => plain.push(c),
        }
    }
    let words: Vec<String> = plain
        .split_whitespace()
        .map(|word| {
            let lower = word.to_ascii_lowercase();
            if lower.contains("://") || lower.starts_with("www.") {
                "[url]".to_string()
            } else if is_token_like(word) {
                "[redacted]".to_string()
            } else {
                word.to_string()
            }
        })
        .collect();
    let joined = words.join(" ");
    if joined.chars().count() <= EXCERPT_CHARS {
        joined
    } else {
        let cut: String = joined.chars().take(EXCERPT_CHARS).collect();
        format!("{cut}…")
    }
}

/// A word that may be a credential: long, and mixing letters with digits or symbols.
fn is_token_like(word: &str) -> bool {
    let core = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    core.len() >= 24
        && core.chars().any(|c| c.is_ascii_digit())
        && core.chars().any(|c| c.is_ascii_alphabetic())
}

/// Check a downloaded part. A file that is text rather than media is deleted (so a
/// later attempt downloads it again) and reported as [`FailureKind::InvalidPlan`]:
/// retrying the same plan would only fetch the same refusal.
pub(crate) async fn reject_non_media(path: &Path, part: usize) -> Result<(), PipelineError> {
    let mut head = Vec::with_capacity(HEAD_BYTES);
    let read = async {
        let file = tokio::fs::File::open(path).await?;
        file.take(HEAD_BYTES as u64).read_to_end(&mut head).await
    }
    .await;
    if let Err(e) = read {
        // Unreadable here will fail the next stage with a proper I/O error.
        log::warn!("pipeline: could not inspect {}: {e}", path.display());
        return Ok(());
    }
    if looks_like_media(&head) || !looks_like_text(&head) {
        return Ok(());
    }
    let size = tokio::fs::metadata(path).await.map(|m| m.len()).unwrap_or(0);
    let quote = excerpt(&head);
    log::warn!(
        "pipeline: part {part} ({}, {size} bytes) is not audio; the server sent: {quote}",
        path.display()
    );
    if let Err(e) = tokio::fs::remove_file(path).await {
        log::warn!("pipeline: could not delete {}: {e}", path.display());
    }
    let what = if quote.is_empty() { "an empty file".to_string() } else { format!("\"{quote}\"") };
    Err(PipelineError::new(
        FailureKind::InvalidPlan,
        format!(
            "The download for part {part} is not audio ({size} bytes): the server sent {what}. \
             The store most likely refused this download."
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_signatures_are_recognised() {
        let mut mp4 = vec![0, 0, 0, 0x20];
        mp4.extend_from_slice(b"ftypaax ");
        assert!(looks_like_media(&mp4));
        assert!(looks_like_media(b"ID3\x04\x00\x00"));
        assert!(looks_like_media(&[0xFF, 0xFB, 0x90, 0x00]));
        assert!(looks_like_media(&[0xFF, 0xF1, 0x50, 0x80])); // ADTS AAC
        assert!(looks_like_media(b"PK\x03\x04rest"));
        assert!(!looks_like_media(b"<html>"));
        assert!(!looks_like_media(b"{\"error\":1}"));
    }

    #[test]
    fn text_bodies_are_text_and_binary_is_not() {
        assert!(looks_like_text(b"<?xml version=\"1.0\"?><Error/>"));
        assert!(looks_like_text(b"  \n<!DOCTYPE html><html>"));
        assert!(looks_like_text(b"{\"message\":\"denied\"}"));
        assert!(looks_like_text(b"[1,2]"));
        assert!(looks_like_text(b"Access denied for this title"));
        assert!(looks_like_text(b""));
        assert!(!looks_like_text(&[0x00, 0x01, 0x02, 0x41, 0x42, 0x43, 0x9F, 0x00]));
        let mostly_binary: Vec<u8> = (0..=255u8).cycle().take(1024).collect();
        assert!(!looks_like_text(&mostly_binary));
    }

    #[test]
    fn excerpt_strips_markup_and_hides_urls_and_tokens() {
        let body = b"<html><head><title>403 Forbidden</title></head><body>\n\
            <p>Request https://cds.audible.com/download?asin=B0X&token=abc denied.</p>\n\
            <p>Ref: Zq8x7Rk2Lm4Np6Qs9Tv1Wx3Yz5Ab</p></body></html>";
        let quote = excerpt(body);
        assert_eq!(quote, "403 Forbidden Request [url] denied. Ref: [redacted]");
    }

    #[test]
    fn excerpt_is_bounded() {
        let body = "word ".repeat(200);
        let quote = excerpt(body.as_bytes());
        assert!(quote.chars().count() <= EXCERPT_CHARS + 1, "{quote}");
        assert!(quote.ends_with('…'));
    }

    #[tokio::test]
    async fn a_text_download_is_rejected_and_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.aax");
        std::fs::write(&path, b"<?xml version=\"1.0\"?><Error><Code>AccessDenied</Code></Error>")
            .unwrap();
        let err = reject_non_media(&path, 1).await.expect_err("text is not audio");
        assert_eq!(err.kind, FailureKind::InvalidPlan);
        assert!(!err.is_retryable());
        assert!(err.message.contains("AccessDenied"), "{}", err.message);
        assert!(!path.exists(), "the bogus file is deleted");
    }

    #[tokio::test]
    async fn a_media_download_passes_and_stays() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.mp3");
        std::fs::write(&path, b"ID3\x04\x00\x00\x00\x00\x00\x00rest of the file").unwrap();
        reject_non_media(&path, 1).await.unwrap();
        assert!(path.exists());
    }
}
