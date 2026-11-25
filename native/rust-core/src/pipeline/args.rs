//! Tag sets and ffmpeg argument vectors.
//!
//! Port of the command construction in `triggerConversion` (decrypt/join with tags and
//! cover) and `encodeChapterToMp3` in
//! `modules/expo-rust-bridge/android/.../DownloadOrchestrator.kt`, with its known bugs
//! fixed:
//! - values are passed raw in an argv; Android's `escapeMetadata` existed only for
//!   FFmpeg-Kit's string parser and left stray backslashes in tags;
//! - `album` is the book title (Audiobookshelf reads `album` as the title), not
//!   `"<Series>, Book 3.0"`;
//! - series go in `show`/`episode_id` (M4B) or `series`/`series-part` (MP3), which is
//!   where Audiobookshelf looks, with the sequence exactly as the store gave it;
//! - `copyright` is written only when both a year and a publisher are known (Android
//!   invented the year 2024);
//! - ASIN/ISBN are not stuffed into `grouping` (Audiobookshelf doesn't read them there;
//!   they go in MP3 TXXX frames and in `metadata.opf`).
//!
//! The mp4 muxer only writes the iTunes atoms it knows, so `publisher`, `asin`, `isbn`
//! and `language` cannot be stored in an M4B without switching the whole file to
//! QuickTime `mdta` keys (which iTunes-style readers ignore). Those reach
//! Audiobookshelf through `metadata.opf`.

use std::ffi::OsString;
use std::path::Path;

use super::BookMeta;

/// `(ffmpeg metadata key, value)` pairs.
pub(crate) type Tags = Vec<(&'static str, String)>;

/// MP3 bitrate bounds (kbps). Android's floor was 48 and an unknown bitrate fell to
/// it; 64 is the floor here and the value used when the source bitrate is unknown.
pub(crate) const MP3_MIN_KBPS: u32 = 64;
pub(crate) const MP3_MAX_KBPS: u32 = 128;

/// MP3 bitrate for a source of `source_bps` bits/s: its own rate in kbps, clamped to
/// [`MP3_MIN_KBPS`]..=[`MP3_MAX_KBPS`]; unknown or zero → [`MP3_MIN_KBPS`].
pub(crate) fn mp3_bitrate_kbps(source_bps: Option<u64>) -> u32 {
    match source_bps {
        Some(bps) if bps > 0 => {
            let kbps = (bps / 1000).min(u64::from(u32::MAX)) as u32;
            kbps.clamp(MP3_MIN_KBPS, MP3_MAX_KBPS)
        }
        _ => MP3_MIN_KBPS,
    }
}

/// The year of a `YYYY-MM-DD` / `YYYY` date, if it starts with four digits.
pub(crate) fn year_of(date: Option<&str>) -> Option<String> {
    let date = date?.trim();
    let year = date.get(..4)?;
    year.chars().all(|c| c.is_ascii_digit()).then(|| year.to_string())
}

fn non_empty(value: Option<&String>) -> Option<String> {
    value.map(|v| v.trim()).filter(|v| !v.is_empty()).map(str::to_string)
}

fn joined(values: &[String]) -> Option<String> {
    let list: Vec<&str> = values.iter().map(|v| v.trim()).filter(|v| !v.is_empty()).collect();
    (!list.is_empty()).then(|| list.join(", "))
}

/// Genres joined with `;` (one of the separators Audiobookshelf splits on), or
/// `Audiobook` as Android wrote when there are none.
fn genre(meta: &BookMeta) -> String {
    let list: Vec<&str> = meta.genres.iter().map(|g| g.trim()).filter(|g| !g.is_empty()).collect();
    if list.is_empty() {
        "Audiobook".to_string()
    } else {
        list.join("; ")
    }
}

/// Android's `comment`: the description, with the subtitle appended when there is one.
fn comment(meta: &BookMeta) -> Option<String> {
    let description = non_empty(meta.description.as_ref());
    match (description, non_empty(meta.subtitle.as_ref())) {
        (Some(d), Some(s)) => Some(format!("{d}\n\nSubtitle: {s}")),
        (None, Some(s)) => Some(format!("Subtitle: {s}")),
        (d, None) => d,
    }
}

/// Book-level tags for an M4B (iTunes atoms the mp4 muxer writes).
pub(crate) fn m4b_tags(meta: &BookMeta) -> Tags {
    let mut tags: Tags = Vec::new();
    tags.push(("title", meta.title.clone()));
    tags.push(("album", meta.title.clone()));
    if let Some(authors) = joined(&meta.authors) {
        tags.push(("artist", authors.clone()));
        tags.push(("album_artist", authors));
    }
    if let Some(narrators) = joined(&meta.narrators) {
        tags.push(("composer", narrators));
    }
    if let Some(series) = non_empty(meta.series.as_ref()) {
        tags.push(("show", series));
        if let Some(seq) = non_empty(meta.series_sequence.as_ref()) {
            tags.push(("episode_id", seq));
        }
    }
    let year = year_of(meta.release_date.as_deref());
    if let Some(year) = &year {
        tags.push(("date", year.clone()));
    }
    if let (Some(year), Some(publisher)) = (&year, non_empty(meta.publisher.as_ref())) {
        tags.push(("copyright", format!("©{year} {publisher};(P){year} {publisher}")));
    }
    if let Some(description) = non_empty(meta.description.as_ref()) {
        tags.push(("description", description));
    }
    if let Some(comment) = comment(meta) {
        tags.push(("comment", comment));
    }
    tags.push(("genre", genre(meta)));
    // iTunes media kind 2 = Audiobook (what Audible's own files carry).
    tags.push(("media_type", "2".to_string()));
    tags
}

/// Book-level ID3v2.3 tags shared by every MP3 the pipeline writes. Keys without a
/// standard frame (`series`, `series-part`, `asin`, `isbn`, `description`) become TXXX
/// frames named after the key, which is how Audiobookshelf reads them back.
fn mp3_common_tags(meta: &BookMeta) -> Tags {
    let mut tags: Tags = Vec::new();
    tags.push(("album", meta.title.clone()));
    if let Some(authors) = joined(&meta.authors) {
        tags.push(("artist", authors.clone()));
        tags.push(("album_artist", authors));
    }
    if let Some(narrators) = joined(&meta.narrators) {
        tags.push(("composer", narrators));
    }
    if let Some(series) = non_empty(meta.series.as_ref()) {
        tags.push(("series", series));
        if let Some(seq) = non_empty(meta.series_sequence.as_ref()) {
            tags.push(("series-part", seq));
        }
    }
    if let Some(year) = year_of(meta.release_date.as_deref()) {
        tags.push(("date", year));
    }
    if let Some(publisher) = non_empty(meta.publisher.as_ref()) {
        tags.push(("publisher", publisher));
    }
    if let Some(asin) = non_empty(meta.asin.as_ref()) {
        tags.push(("asin", asin));
    }
    if let Some(isbn) = non_empty(meta.isbn.as_ref()) {
        tags.push(("isbn", isbn));
    }
    if let Some(description) = non_empty(meta.description.as_ref()) {
        tags.push(("description", description));
    }
    tags.push(("genre", genre(meta)));
    tags
}

/// Tags for chapter `n` of `total` in MP3-per-chapter output. `title` is the chapter
/// name: Audiobookshelf uses per-file titles as chapter names only when they are all
/// set, unique and differ from the book title.
pub(crate) fn mp3_chapter_tags(meta: &BookMeta, title: &str, n: usize, total: usize) -> Tags {
    let mut tags: Tags = vec![("title", title.to_string())];
    tags.extend(mp3_common_tags(meta));
    tags.push(("track", format!("{n}/{total}")));
    tags
}

/// Tags for a whole book stored as one MP3.
pub(crate) fn mp3_book_tags(meta: &BookMeta) -> Tags {
    let mut tags: Tags = vec![("title", meta.title.clone())];
    tags.extend(mp3_common_tags(meta));
    tags
}

/// `-metadata key=value` pairs. Values are raw (argv, no shell); NULs, which no argv
/// can carry, are dropped.
pub(crate) fn metadata_args(tags: &[(&'static str, String)]) -> Vec<OsString> {
    let mut args = Vec::with_capacity(tags.len() * 2);
    for (key, value) in tags {
        args.push(OsString::from("-metadata"));
        args.push(OsString::from(format!("{key}={}", value.replace('\0', ""))));
    }
    args
}

/// Audible DRM for a decrypting input.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Drm<'a> {
    /// AAXC: hex key and IV from the licence voucher.
    Aaxc { key: &'a str, iv: &'a str },
    /// AAX: the account's hex activation bytes.
    Aax { activation_bytes: &'a str },
}

/// Demuxer options that must precede `-i` for an encrypted input (also accepted by
/// ffprobe).
pub(crate) fn drm_input_args(drm: Drm<'_>) -> Vec<OsString> {
    match drm {
        Drm::Aaxc { key, iv } => vec![
            "-audible_key".into(),
            key.into(),
            "-audible_iv".into(),
            iv.into(),
        ],
        Drm::Aax { activation_bytes } => {
            vec!["-activation_bytes".into(), activation_bytes.into()]
        }
    }
}

/// How the primary audio enters ffmpeg.
#[derive(Debug, Clone)]
pub(crate) enum Input<'a> {
    /// One file, with optional demuxer options (DRM keys) before `-i`.
    Single { pre: Vec<OsString>, path: &'a Path },
    /// A concat-demuxer list (lossless join; all parts share codec parameters).
    Concat { list: &'a Path },
    /// Several files decoded and concatenated by the `concat` filter (parts whose
    /// codecs or parameters differ, so a stream copy is impossible).
    ConcatFilter { paths: Vec<&'a Path> },
}

impl Input<'_> {
    fn count(&self) -> usize {
        match self {
            Input::ConcatFilter { paths } => paths.len(),
            _ => 1,
        }
    }
}

/// Where the output's global tags come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetaSource {
    /// Keep the source's own tags; `-metadata` overrides individual keys.
    KeepSource,
    /// Start from nothing (joins: the first part's tags must not leak; per-chapter MP3s).
    Reset,
}

/// Where the output's chapters come from.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ChapterSource<'a> {
    KeepSource,
    /// An FFMETADATA chapter file.
    File(&'a Path),
    Drop,
}

/// Cover picture input.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Cover<'a> {
    pub path: &'a Path,
    /// JPEG/PNG go in as-is; anything else is re-encoded to MJPEG.
    pub copy: bool,
}

/// Output audio codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AudioCodec {
    Copy,
    Aac { kbps: u32 },
    Mp3 { kbps: u32 },
}

/// Output container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Container {
    /// `.m4b` (ipod muxer), moov moved to the front so players can stream it.
    M4b,
    /// `.mp3` with ID3v2.3 tags.
    Mp3,
}

impl Container {
    pub(crate) fn extension(self) -> &'static str {
        match self {
            Container::M4b => "m4b",
            Container::Mp3 => "mp3",
        }
    }
}

/// Everything about the output side of one ffmpeg run.
#[derive(Debug, Clone)]
pub(crate) struct OutputSpec<'a> {
    pub metadata: MetaSource,
    pub chapters: ChapterSource<'a>,
    pub cover: Option<Cover<'a>>,
    /// Without an external cover, carry over the source's own picture (if any).
    pub keep_source_cover: bool,
    pub tags: Tags,
    pub audio: AudioCodec,
    pub container: Container,
    /// Input seek/duration in seconds (per-chapter MP3).
    pub segment: Option<(f64, f64)>,
}

/// Build a complete ffmpeg argv (everything after the program name, before the runner's
/// common flags) for `input` → `output`.
pub(crate) fn ffmpeg_args(input: &Input<'_>, spec: &OutputSpec<'_>, output: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();
    let push = |a: &mut Vec<OsString>, s: &str| a.push(OsString::from(s));

    // --- inputs ---
    match input {
        Input::Single { pre, path } => {
            if let Some((start, _)) = spec.segment {
                push(&mut args, "-ss");
                push(&mut args, &format!("{start:.3}"));
            }
            args.extend(pre.iter().cloned());
            push(&mut args, "-i");
            args.push(path.as_os_str().to_owned());
        }
        Input::Concat { list } => {
            for s in ["-f", "concat", "-safe", "0", "-i"] {
                push(&mut args, s);
            }
            args.push(list.as_os_str().to_owned());
        }
        Input::ConcatFilter { paths } => {
            for path in paths {
                push(&mut args, "-i");
                args.push(path.as_os_str().to_owned());
            }
        }
    }
    let mut next_index = input.count();
    let chapters_index = match spec.chapters {
        ChapterSource::File(path) => {
            for s in ["-f", "ffmetadata", "-i"] {
                push(&mut args, s);
            }
            args.push(path.as_os_str().to_owned());
            next_index += 1;
            Some(next_index - 1)
        }
        _ => None,
    };
    let cover_index = spec.cover.map(|cover| {
        push(&mut args, "-i");
        args.push(cover.path.as_os_str().to_owned());
        next_index += 1;
        next_index - 1
    });
    if let Some((_, duration)) = spec.segment {
        push(&mut args, "-t");
        push(&mut args, &format!("{duration:.3}"));
    }

    // --- metadata and chapters ---
    match (spec.metadata, spec.chapters, chapters_index) {
        // Chapter titles travel with the global-metadata mapping: map from the chapter
        // file itself (it has no global tags) to reset, or from input 0 to keep.
        (MetaSource::Reset, ChapterSource::File(_), Some(idx)) => {
            push(&mut args, "-map_metadata");
            push(&mut args, &idx.to_string());
            push(&mut args, "-map_chapters");
            push(&mut args, &idx.to_string());
        }
        (MetaSource::KeepSource, ChapterSource::File(_), Some(idx)) => {
            push(&mut args, "-map_metadata");
            push(&mut args, "0");
            push(&mut args, "-map_chapters");
            push(&mut args, &idx.to_string());
        }
        (MetaSource::Reset, _, _) => {
            for s in ["-map_metadata", "-1", "-map_chapters", "-1"] {
                push(&mut args, s);
            }
        }
        (MetaSource::KeepSource, ChapterSource::Drop, _) => {
            push(&mut args, "-map_chapters");
            push(&mut args, "-1");
        }
        (MetaSource::KeepSource, _, _) => {}
    }

    // --- streams ---
    match input {
        Input::ConcatFilter { paths } => {
            let labels: String = (0..paths.len()).map(|i| format!("[{i}:a:0]")).collect();
            push(&mut args, "-filter_complex");
            push(
                &mut args,
                &format!("{labels}concat=n={}:v=0:a=1[a]", paths.len()),
            );
            push(&mut args, "-map");
            push(&mut args, "[a]");
        }
        _ => {
            push(&mut args, "-map");
            push(&mut args, "0:a:0");
        }
    }
    match (spec.cover, cover_index) {
        (Some(cover), Some(idx)) => {
            push(&mut args, "-map");
            push(&mut args, &format!("{idx}:v:0"));
            push(&mut args, "-c:v");
            push(&mut args, if cover.copy { "copy" } else { "mjpeg" });
            push(&mut args, "-disposition:v:0");
            push(&mut args, "attached_pic");
        }
        _ if spec.keep_source_cover && matches!(input, Input::Single { .. }) => {
            push(&mut args, "-map");
            push(&mut args, "0:v:0?");
            push(&mut args, "-c:v");
            push(&mut args, "copy");
        }
        _ => {}
    }

    match spec.audio {
        AudioCodec::Copy => {
            push(&mut args, "-c:a");
            push(&mut args, "copy");
        }
        AudioCodec::Aac { kbps } => {
            push(&mut args, "-c:a");
            push(&mut args, "aac");
            push(&mut args, "-b:a");
            push(&mut args, &format!("{kbps}k"));
        }
        AudioCodec::Mp3 { kbps } => {
            push(&mut args, "-c:a");
            push(&mut args, "libmp3lame");
            push(&mut args, "-b:a");
            push(&mut args, &format!("{kbps}k"));
        }
    }

    args.extend(metadata_args(&spec.tags));

    match spec.container {
        Container::M4b => {
            for s in ["-movflags", "+faststart", "-f", "ipod"] {
                push(&mut args, s);
            }
        }
        Container::Mp3 => {
            for s in ["-id3v2_version", "3", "-write_id3v1", "0", "-f", "mp3"] {
                push(&mut args, s);
            }
        }
    }
    args.push(output.as_os_str().to_owned());
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(args: &[OsString]) -> Vec<String> {
        args.iter().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    fn meta() -> BookMeta {
        BookMeta {
            product_id: "B0TEST".into(),
            title: "The \"Quoted\" Book".into(),
            subtitle: Some("A Subtitle".into()),
            authors: vec!["Ann Author".into(), "Bob Writer".into()],
            narrators: vec!["Nora Narrator".into()],
            series: Some("Saga".into()),
            series_sequence: Some("2.5".into()),
            publisher: Some("Pub House".into()),
            release_date: Some("2019-05-01".into()),
            description: Some("Line one\nback\\slash = ok".into()),
            language: Some("english".into()),
            genres: vec!["Fantasy".into(), "Epic".into()],
            asin: Some("B0TEST".into()),
            isbn: Some("9780000000001".into()),
            cover_url: None,
        }
    }

    fn get<'a>(tags: &'a Tags, key: &str) -> Option<&'a str> {
        tags.iter().find(|(k, _)| *k == key).map(|(_, v)| v.as_str())
    }

    #[test]
    fn bitrate_is_clamped_with_a_64k_floor() {
        assert_eq!(mp3_bitrate_kbps(None), 64);
        assert_eq!(mp3_bitrate_kbps(Some(0)), 64);
        assert_eq!(mp3_bitrate_kbps(Some(32_000)), 64);
        assert_eq!(mp3_bitrate_kbps(Some(96_500)), 96);
        assert_eq!(mp3_bitrate_kbps(Some(320_000)), 128);
    }

    #[test]
    fn year_parsing() {
        assert_eq!(year_of(Some("2019-05-01")).as_deref(), Some("2019"));
        assert_eq!(year_of(Some("1999")).as_deref(), Some("1999"));
        assert_eq!(year_of(Some("May 2019")), None);
        assert_eq!(year_of(None), None);
    }

    #[test]
    fn m4b_tags_fix_the_android_quirks() {
        let tags = m4b_tags(&meta());
        assert_eq!(get(&tags, "title"), Some("The \"Quoted\" Book"));
        // The title, not "<Series>, Book 2.5" — Audiobookshelf reads album as the title.
        assert_eq!(get(&tags, "album"), Some("The \"Quoted\" Book"));
        assert_eq!(get(&tags, "artist"), Some("Ann Author, Bob Writer"));
        assert_eq!(get(&tags, "album_artist"), Some("Ann Author, Bob Writer"));
        assert_eq!(get(&tags, "composer"), Some("Nora Narrator"));
        assert_eq!(get(&tags, "show"), Some("Saga"));
        assert_eq!(get(&tags, "episode_id"), Some("2.5"));
        assert_eq!(get(&tags, "date"), Some("2019"));
        assert_eq!(get(&tags, "copyright"), Some("©2019 Pub House;(P)2019 Pub House"));
        assert_eq!(get(&tags, "genre"), Some("Fantasy; Epic"));
        assert_eq!(get(&tags, "media_type"), Some("2"));
        assert_eq!(
            get(&tags, "comment"),
            Some("Line one\nback\\slash = ok\n\nSubtitle: A Subtitle")
        );
        assert_eq!(get(&tags, "grouping"), None);
    }

    #[test]
    fn copyright_needs_a_real_year() {
        let mut m = meta();
        m.release_date = None;
        let tags = m4b_tags(&m);
        assert_eq!(get(&tags, "copyright"), None);
        assert_eq!(get(&tags, "date"), None);
    }

    #[test]
    fn minimal_meta_still_tags_title_and_genre() {
        let m = BookMeta { product_id: "x".into(), title: "T".into(), ..Default::default() };
        let tags = m4b_tags(&m);
        assert_eq!(get(&tags, "title"), Some("T"));
        assert_eq!(get(&tags, "genre"), Some("Audiobook"));
        assert_eq!(get(&tags, "artist"), None);
        assert_eq!(get(&tags, "show"), None);
    }

    #[test]
    fn mp3_chapter_tags_identify_the_chapter_and_the_book() {
        let tags = mp3_chapter_tags(&meta(), "Chapter Three", 3, 12);
        assert_eq!(get(&tags, "title"), Some("Chapter Three"));
        assert_eq!(get(&tags, "album"), Some("The \"Quoted\" Book"));
        assert_eq!(get(&tags, "track"), Some("3/12"));
        assert_eq!(get(&tags, "artist"), Some("Ann Author, Bob Writer"));
        assert_eq!(get(&tags, "series"), Some("Saga"));
        assert_eq!(get(&tags, "series-part"), Some("2.5"));
        assert_eq!(get(&tags, "asin"), Some("B0TEST"));
        assert_eq!(get(&tags, "isbn"), Some("9780000000001"));
        assert_eq!(get(&tags, "publisher"), Some("Pub House"));
    }

    #[test]
    fn metadata_values_are_passed_raw() {
        let args = strs(&metadata_args(&[
            ("title", "A \"quote\" and \\ back=slash".to_string()),
            ("comment", "nul\0byte".to_string()),
        ]));
        assert_eq!(
            args,
            vec![
                "-metadata",
                "title=A \"quote\" and \\ back=slash",
                "-metadata",
                "comment=nulbyte"
            ]
        );
    }

    #[test]
    fn aaxc_decrypt_args() {
        let pre = drm_input_args(Drm::Aaxc { key: "00112233445566778899aabbccddeeff", iv: "ffeeddccbbaa99887766554433221100" });
        let cover = Cover { path: Path::new("/w/cover.jpg"), copy: true };
        let spec = OutputSpec {
            metadata: MetaSource::KeepSource,
            chapters: ChapterSource::KeepSource,
            cover: Some(cover),
            keep_source_cover: true,
            tags: vec![("title", "T".into())],
            audio: AudioCodec::Copy,
            container: Container::M4b,
            segment: None,
        };
        let args = strs(&ffmpeg_args(
            &Input::Single { pre, path: Path::new("/w/B0.aaxc") },
            &spec,
            Path::new("/d/.librisync-tmp.T.m4b"),
        ));
        assert_eq!(
            args,
            vec![
                "-audible_key", "00112233445566778899aabbccddeeff",
                "-audible_iv", "ffeeddccbbaa99887766554433221100",
                "-i", "/w/B0.aaxc",
                "-i", "/w/cover.jpg",
                "-map", "0:a:0",
                "-map", "1:v:0", "-c:v", "copy", "-disposition:v:0", "attached_pic",
                "-c:a", "copy",
                "-metadata", "title=T",
                "-movflags", "+faststart", "-f", "ipod",
                "/d/.librisync-tmp.T.m4b",
            ]
        );
    }

    #[test]
    fn aax_decrypt_args_without_cover_keep_the_source_picture() {
        let pre = drm_input_args(Drm::Aax { activation_bytes: "1a2b3c4d" });
        let spec = OutputSpec {
            metadata: MetaSource::KeepSource,
            chapters: ChapterSource::KeepSource,
            cover: None,
            keep_source_cover: true,
            tags: vec![],
            audio: AudioCodec::Copy,
            container: Container::M4b,
            segment: None,
        };
        let args = strs(&ffmpeg_args(
            &Input::Single { pre, path: Path::new("/w/B0.aax") },
            &spec,
            Path::new("/w/decrypted.m4b"),
        ));
        assert_eq!(
            args,
            vec![
                "-activation_bytes", "1a2b3c4d",
                "-i", "/w/B0.aax",
                "-map", "0:a:0",
                "-map", "0:v:0?", "-c:v", "copy",
                "-c:a", "copy",
                "-movflags", "+faststart", "-f", "ipod",
                "/w/decrypted.m4b",
            ]
        );
    }

    #[test]
    fn join_args_take_chapters_and_tags_from_the_chapter_file() {
        let spec = OutputSpec {
            metadata: MetaSource::Reset,
            chapters: ChapterSource::File(Path::new("/w/chapters.ffmeta")),
            cover: Some(Cover { path: Path::new("/w/cover.png"), copy: true }),
            keep_source_cover: false,
            tags: vec![],
            audio: AudioCodec::Copy,
            container: Container::M4b,
            segment: None,
        };
        let args = strs(&ffmpeg_args(
            &Input::Concat { list: Path::new("/w/join.txt") },
            &spec,
            Path::new("/w/joined.m4b"),
        ));
        assert_eq!(
            args[..12],
            [
                "-f", "concat", "-safe", "0", "-i", "/w/join.txt",
                "-f", "ffmetadata", "-i", "/w/chapters.ffmeta",
                "-i", "/w/cover.png",
            ]
        );
        let joined = args.join(" ");
        assert!(joined.contains("-map_metadata 1 -map_chapters 1"));
        assert!(joined.contains("-map 0:a:0 -map 2:v:0"));
    }

    #[test]
    fn mixed_join_uses_the_concat_filter_and_reencodes() {
        let paths = [Path::new("/w/1.m4a"), Path::new("/w/2.mp3")];
        let spec = OutputSpec {
            metadata: MetaSource::Reset,
            chapters: ChapterSource::File(Path::new("/w/c.ffmeta")),
            cover: None,
            keep_source_cover: false,
            tags: vec![],
            audio: AudioCodec::Aac { kbps: 96 },
            container: Container::M4b,
            segment: None,
        };
        let args = strs(&ffmpeg_args(
            &Input::ConcatFilter { paths: paths.to_vec() },
            &spec,
            Path::new("/w/joined.m4b"),
        ))
        .join(" ");
        assert!(args.starts_with("-i /w/1.m4a -i /w/2.mp3 -f ffmetadata -i /w/c.ffmeta"));
        assert!(args.contains("-map_metadata 2 -map_chapters 2"));
        assert!(args.contains("-filter_complex [0:a:0][1:a:0]concat=n=2:v=0:a=1[a] -map [a]"));
        assert!(args.contains("-c:a aac -b:a 96k"));
    }

    #[test]
    fn chapter_mp3_args_seek_and_reset_metadata() {
        let spec = OutputSpec {
            metadata: MetaSource::Reset,
            chapters: ChapterSource::Drop,
            cover: Some(Cover { path: Path::new("/w/cover.webp"), copy: false }),
            keep_source_cover: false,
            tags: vec![("title", "One".into()), ("track", "1/3".into())],
            audio: AudioCodec::Mp3 { kbps: 64 },
            container: Container::Mp3,
            segment: Some((12.5, 60.25)),
        };
        let args = strs(&ffmpeg_args(
            &Input::Single { pre: vec![], path: Path::new("/w/book.m4b") },
            &spec,
            Path::new("/d/.librisync-tmp.01 - One.mp3"),
        ));
        assert_eq!(
            args,
            vec![
                "-ss", "12.500", "-i", "/w/book.m4b",
                "-i", "/w/cover.webp",
                "-t", "60.250",
                "-map_metadata", "-1", "-map_chapters", "-1",
                "-map", "0:a:0",
                "-map", "1:v:0", "-c:v", "mjpeg", "-disposition:v:0", "attached_pic",
                "-c:a", "libmp3lame", "-b:a", "64k",
                "-metadata", "title=One", "-metadata", "track=1/3",
                "-id3v2_version", "3", "-write_id3v1", "0", "-f", "mp3",
                "/d/.librisync-tmp.01 - One.mp3",
            ]
        );
    }
}
