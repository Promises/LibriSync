//! Cover download and the Audiobookshelf sidecar files: `cover.jpg`, `desc.txt`,
//! `reader.txt` and `metadata.opf`.
//!
//! The cover download ports `DownloadOrchestrator.kt`'s cover fetch (failure is never
//! fatal: the book is written without one). The sidecars are new — Android has none —
//! and follow what Audiobookshelf's scanner reads (`OpfFileScanner`, desc/reader txt).

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use tokio_util::sync::CancellationToken;

use super::fsutil::io_error;
use super::{BookMeta, PipelineError, WORK_PREFIX};

const COVER_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_COVER_BYTES: usize = 20 * 1024 * 1024;

/// A downloaded cover image in the work dir.
#[derive(Debug, Clone)]
pub(crate) struct CoverImage {
    pub path: PathBuf,
    /// `jpg`, `png` or `webp`.
    pub extension: &'static str,
}

impl CoverImage {
    /// JPEG and PNG can be stream-copied into MP4 `covr` and ID3 `APIC`; others are
    /// re-encoded.
    pub(crate) fn can_copy(&self) -> bool {
        matches!(self.extension, "jpg" | "png")
    }
}

/// Image type from magic bytes.
pub(crate) fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

/// Download `url` into `work_dir`. Any failure is logged and yields `None`.
pub(crate) async fn fetch_cover(
    http: &reqwest::Client,
    url: &str,
    work_dir: &Path,
    cancel: &CancellationToken,
) -> Option<CoverImage> {
    let download = async {
        let response = http
            .get(url)
            .timeout(COVER_TIMEOUT)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()));
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| e.to_string())?;
            if bytes.len() + chunk.len() > MAX_COVER_BYTES {
                return Err("image larger than 20 MB".to_string());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    };
    let bytes = tokio::select! {
        _ = cancel.cancelled() => return None,
        result = download => result,
    };
    let bytes = match bytes {
        Ok(bytes) => bytes,
        Err(e) => {
            log::warn!("pipeline: cover download failed, continuing without one: {e}");
            return None;
        }
    };
    let Some(extension) = sniff_image(&bytes) else {
        log::warn!("pipeline: cover at {url} is not a JPEG, PNG or WebP image; skipping it");
        return None;
    };
    let path = work_dir.join(format!("{WORK_PREFIX}cover.{extension}"));
    if let Err(e) = tokio::fs::write(&path, &bytes).await {
        log::warn!("pipeline: could not save the cover: {e}");
        return None;
    }
    Some(CoverImage { path, extension })
}

fn xml_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Characters XML 1.0 cannot carry at all.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

fn present(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

/// `metadata.opf` (OPF 2.0) with the fields Audiobookshelf's OPF scanner reads:
/// title, authors (`aut`) and narrators (`nrt`), publisher, date, language,
/// description, genres (`dc:subject`), ASIN/ISBN identifiers and the Calibre series
/// meta tags.
pub(crate) fn opf(meta: &BookMeta) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("    <dc:title>{}</dc:title>", xml_escape(&meta.title)));
    for author in meta.authors.iter().map(|a| a.trim()).filter(|a| !a.is_empty()) {
        lines.push(format!(
            "    <dc:creator opf:role=\"aut\">{}</dc:creator>",
            xml_escape(author)
        ));
    }
    for narrator in meta.narrators.iter().map(|n| n.trim()).filter(|n| !n.is_empty()) {
        lines.push(format!(
            "    <dc:creator opf:role=\"nrt\">{}</dc:creator>",
            xml_escape(narrator)
        ));
    }
    if let Some(publisher) = present(&meta.publisher) {
        lines.push(format!("    <dc:publisher>{}</dc:publisher>", xml_escape(publisher)));
    }
    if let Some(date) = present(&meta.release_date) {
        lines.push(format!("    <dc:date>{}</dc:date>", xml_escape(date)));
    }
    if let Some(language) = present(&meta.language) {
        lines.push(format!("    <dc:language>{}</dc:language>", xml_escape(language)));
    }
    if let Some(description) = present(&meta.description) {
        lines.push(format!("    <dc:description>{}</dc:description>", xml_escape(description)));
    }
    for genre in meta.genres.iter().map(|g| g.trim()).filter(|g| !g.is_empty()) {
        lines.push(format!("    <dc:subject>{}</dc:subject>", xml_escape(genre)));
    }

    // The package's unique-identifier must name one dc:identifier: the ASIN, else the
    // ISBN, else the store's own product id.
    let asin = present(&meta.asin);
    let isbn = present(&meta.isbn);
    if let Some(asin) = asin {
        lines.push(format!(
            "    <dc:identifier id=\"BookId\" opf:scheme=\"ASIN\">{}</dc:identifier>",
            xml_escape(asin)
        ));
    }
    if let Some(isbn) = isbn {
        let id = if asin.is_none() { " id=\"BookId\"" } else { "" };
        lines.push(format!(
            "    <dc:identifier{id} opf:scheme=\"ISBN\">{}</dc:identifier>",
            xml_escape(isbn)
        ));
    }
    if asin.is_none() && isbn.is_none() {
        lines.push(format!(
            "    <dc:identifier id=\"BookId\" opf:scheme=\"LibriSync\">{}</dc:identifier>",
            xml_escape(&meta.product_id)
        ));
    }
    if let Some(series) = present(&meta.series) {
        lines.push(format!(
            "    <meta name=\"calibre:series\" content=\"{}\"/>",
            xml_escape(series)
        ));
        if let Some(seq) = present(&meta.series_sequence) {
            lines.push(format!(
                "    <meta name=\"calibre:series_index\" content=\"{}\"/>",
                xml_escape(seq)
            ));
        }
    }

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <package xmlns=\"http://www.idpf.org/2007/opf\" version=\"2.0\" unique-identifier=\"BookId\">\n  \
         <metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:opf=\"http://www.idpf.org/2007/opf\" xmlns:calibre=\"http://calibre.kovidgoyal.net/2009/metadata\">\n\
         {}\n  \
         </metadata>\n\
         </package>\n",
        lines.join("\n")
    )
}

/// The text sidecars to write: `(file name, contents)`.
pub(crate) fn text_sidecars(meta: &BookMeta) -> Vec<(&'static str, String)> {
    let mut files = Vec::new();
    if let Some(description) = present(&meta.description) {
        files.push(("desc.txt", format!("{description}\n")));
    }
    let narrators: Vec<&str> =
        meta.narrators.iter().map(|n| n.trim()).filter(|n| !n.is_empty()).collect();
    if !narrators.is_empty() {
        files.push(("reader.txt", format!("{}\n", narrators.join(", "))));
    }
    files.push(("metadata.opf", opf(meta)));
    files
}

/// Write `contents` to `path`.
pub(crate) async fn write_file(path: &Path, contents: &[u8]) -> Result<(), PipelineError> {
    tokio::fs::write(path, contents)
        .await
        .map_err(|e| io_error(format_args!("writing {}", path.display()), e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> BookMeta {
        BookMeta {
            product_id: "B0TEST".into(),
            title: "Kings & <Queens>".into(),
            authors: vec!["Ann Author".into(), "Bob \"B\" Writer".into()],
            narrators: vec!["Nora Narrator".into(), "Ned".into()],
            series: Some("Saga".into()),
            series_sequence: Some("2.5".into()),
            publisher: Some("Pub House".into()),
            release_date: Some("2019-05-01".into()),
            description: Some("A tale.\nTwo lines.".into()),
            language: Some("English".into()),
            genres: vec!["Fantasy".into()],
            asin: Some("B0TEST".into()),
            isbn: Some("9780000000001".into()),
            ..Default::default()
        }
    }

    #[test]
    fn opf_carries_the_fields_audiobookshelf_reads() {
        let xml = opf(&meta());
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<package"));
        assert!(xml.contains("unique-identifier=\"BookId\""));
        assert!(xml.contains("<dc:title>Kings &amp; &lt;Queens&gt;</dc:title>"));
        assert!(xml.contains("<dc:creator opf:role=\"aut\">Ann Author</dc:creator>"));
        assert!(xml.contains("<dc:creator opf:role=\"aut\">Bob &quot;B&quot; Writer</dc:creator>"));
        assert!(xml.contains("<dc:creator opf:role=\"nrt\">Nora Narrator</dc:creator>"));
        assert!(xml.contains("<dc:creator opf:role=\"nrt\">Ned</dc:creator>"));
        assert!(xml.contains("<dc:publisher>Pub House</dc:publisher>"));
        assert!(xml.contains("<dc:date>2019-05-01</dc:date>"));
        assert!(xml.contains("<dc:language>English</dc:language>"));
        assert!(xml.contains("<dc:description>A tale.\nTwo lines.</dc:description>"));
        assert!(xml.contains("<dc:subject>Fantasy</dc:subject>"));
        assert!(xml.contains("<dc:identifier id=\"BookId\" opf:scheme=\"ASIN\">B0TEST</dc:identifier>"));
        assert!(xml.contains("<dc:identifier opf:scheme=\"ISBN\">9780000000001</dc:identifier>"));
        assert!(xml.contains("<meta name=\"calibre:series\" content=\"Saga\"/>"));
        assert!(xml.contains("<meta name=\"calibre:series_index\" content=\"2.5\"/>"));
        assert!(xml.trim_end().ends_with("</package>"));
    }

    #[test]
    fn opf_without_store_ids_uses_the_product_id() {
        let m = BookMeta {
            product_id: "chirp_42".into(),
            title: "T".into(),
            ..Default::default()
        };
        let xml = opf(&m);
        assert!(xml.contains(
            "<dc:identifier id=\"BookId\" opf:scheme=\"LibriSync\">chirp_42</dc:identifier>"
        ));
        assert!(!xml.contains("calibre:series"));
        assert!(!xml.contains("dc:creator"));
    }

    #[test]
    fn opf_with_only_isbn_names_it_the_book_id() {
        let m = BookMeta {
            product_id: "978".into(),
            title: "T".into(),
            isbn: Some("978".into()),
            ..Default::default()
        };
        assert!(opf(&m).contains("<dc:identifier id=\"BookId\" opf:scheme=\"ISBN\">978</dc:identifier>"));
    }

    #[test]
    fn text_sidecars_skip_empty_fields() {
        let files = text_sidecars(&meta());
        let names: Vec<&str> = files.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, vec!["desc.txt", "reader.txt", "metadata.opf"]);
        assert_eq!(files[1].1, "Nora Narrator, Ned\n");

        let bare = BookMeta { product_id: "x".into(), title: "T".into(), ..Default::default() };
        let names: Vec<&str> = text_sidecars(&bare).iter().map(|(n, _)| *n).collect();
        assert_eq!(names, vec!["metadata.opf"]);
    }

    #[test]
    fn image_sniffing() {
        assert_eq!(sniff_image(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(sniff_image(b"\x89PNG\r\n\x1a\n...."), Some("png"));
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(sniff_image(b"<html>"), None);
    }
}
