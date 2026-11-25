//! Filesystem helpers: free-space checks, I/O error classification, file-name
//! sanitising and atomic placement of output files.
//!
//! Port of `requireFreeSpace`, `explainFailure` and `sanitizeFileName` in
//! `modules/expo-rust-bridge/android/.../DownloadOrchestrator.kt`. Unlike Android, the
//! check runs against the filesystem each step actually writes to (Android measured
//! the cache volume even when the book went to another one).

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use super::{FailureKind, PipelineError};

/// Headroom kept free on top of a step's own needs (Android's `STORAGE_HEADROOM_BYTES`).
pub(crate) const STORAGE_HEADROOM_BYTES: u64 = 100 * 1024 * 1024;

/// Prefix of every file the pipeline writes into `dest_dir` before renaming it into
/// place. Leading dot: Audiobookshelf and most scanners skip hidden files.
pub(crate) const TEMP_PREFIX: &str = ".librisync-tmp.";

const MIB: u64 = 1024 * 1024;

/// Bytes available to unprivileged users on the filesystem holding `dir`, or `None`
/// where the platform gives no answer.
#[cfg(unix)]
pub(crate) fn available_space(dir: &Path) -> io::Result<Option<u64>> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(dir.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c_path` is a valid NUL-terminated string and `stat` is a properly sized,
    // writable statvfs struct that outlives the call.
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    #[allow(clippy::unnecessary_cast)]
    Ok(Some((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64)))
}

/// Bytes available on the filesystem holding `dir` (unknown on this platform).
#[cfg(not(unix))]
pub(crate) fn available_space(_dir: &Path) -> io::Result<Option<u64>> {
    Ok(None)
}

/// Fail with [`FailureKind::DiskFull`] unless `dir`'s filesystem has `needed` bytes plus
/// [`STORAGE_HEADROOM_BYTES`] free. A filesystem that can't report its free space passes.
pub(crate) fn require_free_space(dir: &Path, needed: u64) -> Result<(), PipelineError> {
    let available = match available_space(dir) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Ok(()),
        Err(e) => {
            log::warn!("pipeline: could not read free space of {}: {e}", dir.display());
            return Ok(());
        }
    };
    let required = needed.saturating_add(STORAGE_HEADROOM_BYTES);
    if available < required {
        return Err(PipelineError::new(
            FailureKind::DiskFull,
            format!(
                "Not enough storage in {}: this book needs about {} MB free, {} MB available.",
                dir.display(),
                required.div_ceil(MIB),
                available / MIB
            ),
        ));
    }
    Ok(())
}

/// Whether an I/O error means the disk is full.
pub(crate) fn is_disk_full(e: &io::Error) -> bool {
    if e.kind() == io::ErrorKind::StorageFull {
        return true;
    }
    #[cfg(unix)]
    {
        if e.raw_os_error() == Some(libc::ENOSPC) || e.raw_os_error() == Some(libc::EDQUOT) {
            return true;
        }
    }
    false
}

/// Classify a local I/O error: ENOSPC/quota → `DiskFull`, anything else → `Io`.
pub(crate) fn io_error(context: impl std::fmt::Display, e: io::Error) -> PipelineError {
    if is_disk_full(&e) {
        PipelineError::new(
            FailureKind::DiskFull,
            format!("Ran out of storage while {context}: {e}"),
        )
    } else {
        PipelineError::new(FailureKind::Io, format!("{context}: {e}"))
    }
}

/// Port of Android's `sanitizeFileName`, used for chapter and output file names:
/// characters that are invalid on common filesystems (and control characters) become a
/// space, whitespace is collapsed, trailing dots are stripped, and the result is capped at
/// 80 characters *and* 120 bytes (Android capped characters only; 80 four-byte characters
/// overflow ext4's 255-byte limit once a prefix and extension are added). Empty → `fallback`.
pub(crate) fn sanitize_file_name(name: &str, fallback: &str) -> String {
    const MAX_CHARS: usize = 80;
    const MAX_BYTES: usize = 120;

    let replaced: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let collapsed = replaced.split_whitespace().collect::<Vec<_>>().join(" ");

    let mut out = String::new();
    for (count, c) in collapsed.chars().enumerate() {
        if count >= MAX_CHARS || out.len() + c.len_utf8() > MAX_BYTES {
            break;
        }
        out.push(c);
    }
    // Truncation may expose trailing spaces or dots; strip both until stable.
    let trimmed = out.trim_end_matches(['.', ' ']).trim_start();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

/// The temp path a final output file is written to before [`Staging::commit`].
pub(crate) fn temp_path_for(final_path: &Path) -> PathBuf {
    let name = final_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    final_path.with_file_name(format!("{TEMP_PREFIX}{name}"))
}

/// Output files written under temp names in `dest_dir`, renamed into place together
/// once everything has succeeded. Dropping an uncommitted `Staging` does nothing; call
/// [`Staging::discard`] on failure.
#[derive(Debug, Default)]
pub(crate) struct Staging {
    /// `(temp, final)` pairs in commit order.
    files: Vec<(PathBuf, PathBuf)>,
}

impl Staging {
    /// Reserve `final_path` and return the temp path to write it to.
    pub(crate) fn stage(&mut self, final_path: PathBuf) -> PathBuf {
        let temp = temp_path_for(&final_path);
        self.files.push((temp.clone(), final_path));
        temp
    }

    /// Take over a temp file written elsewhere (ZIP extraction) for `final_path`.
    pub(crate) fn adopt(&mut self, temp: PathBuf, final_path: PathBuf) {
        self.files.push((temp, final_path));
    }

    /// Rename every temp file into place. A failure part-way leaves the files already
    /// renamed (they are complete) and removes the rest.
    pub(crate) async fn commit(self) -> Result<Vec<PathBuf>, PipelineError> {
        let mut done = Vec::with_capacity(self.files.len());
        for (index, (temp, final_path)) in self.files.iter().enumerate() {
            if let Err(e) = tokio::fs::rename(temp, final_path).await {
                for (rest, _) in &self.files[index..] {
                    let _ = tokio::fs::remove_file(rest).await;
                }
                return Err(io_error(
                    format_args!("moving {} into place", final_path.display()),
                    e,
                ));
            }
            done.push(final_path.clone());
        }
        Ok(done)
    }

    /// Remove every temp file (failure or cancellation).
    pub(crate) async fn discard(&self) {
        for (temp, _) in &self.files {
            let _ = tokio::fs::remove_file(temp).await;
        }
    }
}

/// Audio extensions the pipeline may produce or extract.
pub(crate) const AUDIO_EXTENSIONS: &[&str] = &["mp3", "m4a", "m4b", "ogg", "flac", "opus", "wav"];

fn has_audio_extension(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// `NN - Title.mp3` (at least two leading digits): the chapter-file naming scheme.
pub(crate) fn is_chapter_file_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".mp3") else {
        return false;
    };
    let digits = stem.chars().take_while(|c| c.is_ascii_digit()).count();
    digits >= 2 && stem[digits..].starts_with(" - ")
}

/// Delete audio files a previous run of this book left in `dest_dir` that this run did
/// not rewrite: `{file_stem}.<audio ext>` and `NN - *.mp3` chapter files. Deliberately
/// narrow — files the pipeline would never have written are left alone, so two books
/// sharing a folder by accident don't wipe each other's unrelated files.
pub(crate) async fn remove_stale_outputs(
    dest_dir: &Path,
    file_stem: &str,
    keep: &[PathBuf],
) -> Result<(), PipelineError> {
    let keep: HashSet<&Path> = keep.iter().map(PathBuf::as_path).collect();
    let mut entries = match tokio::fs::read_dir(dest_dir).await {
        Ok(entries) => entries,
        Err(e) => return Err(io_error(format_args!("listing {}", dest_dir.display()), e)),
    };
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| io_error(format_args!("listing {}", dest_dir.display()), e))?
    {
        let path = entry.path();
        if keep.contains(path.as_path()) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_ours = is_chapter_file_name(&name)
            || (has_audio_extension(&name)
                && Path::new(&name).file_stem().and_then(|s| s.to_str()) == Some(file_stem));
        if is_ours && entry.file_type().await.map(|t| t.is_file()).unwrap_or(false) {
            log::info!("pipeline: removing previous output {}", path.display());
            tokio::fs::remove_file(&path)
                .await
                .map_err(|e| io_error(format_args!("removing {}", path.display()), e))?;
        }
    }
    Ok(())
}

/// Remove temp files an interrupted earlier run left in `dest_dir`.
pub(crate) async fn remove_stale_temps(dest_dir: &Path) {
    let Ok(mut entries) = tokio::fs::read_dir(dest_dir).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.file_name().to_string_lossy().starts_with(TEMP_PREFIX) {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
}

/// Place `src` at `dst` without moving it: a hard link when both are on one filesystem
/// (instant, no extra space), else a copy. The source stays in the work dir until the
/// whole run succeeds, so a later failure can retry without downloading again.
pub(crate) async fn link_or_copy(src: &Path, dst: &Path) -> Result<(), PipelineError> {
    let _ = tokio::fs::remove_file(dst).await;
    if tokio::fs::hard_link(src, dst).await.is_ok() {
        return Ok(());
    }
    let size = tokio::fs::metadata(src)
        .await
        .map_err(|e| io_error(format_args!("reading {}", src.display()), e))?
        .len();
    if let Some(parent) = dst.parent() {
        require_free_space(parent, size)?;
    }
    tokio::fs::copy(src, dst)
        .await
        .map_err(|e| io_error(format_args!("copying to {}", dst.display()), e))?;
    Ok(())
}

/// Size of a file, 0 when it can't be read.
pub(crate) async fn file_size(path: &Path) -> u64 {
    tokio::fs::metadata(path).await.map(|m| m.len()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_replaces_invalid_characters_and_collapses_whitespace() {
        assert_eq!(
            sanitize_file_name("Part 1: The \"Beginning\" / End?", "Chapter"),
            "Part 1 The Beginning End"
        );
        assert_eq!(sanitize_file_name("a\tb\nc", "x"), "a b c");
    }

    #[test]
    fn sanitize_strips_trailing_dots_and_falls_back_when_empty() {
        assert_eq!(sanitize_file_name("Epilogue...", "Chapter"), "Epilogue");
        assert_eq!(sanitize_file_name(" ?* ", "Chapter 3"), "Chapter 3");
        assert_eq!(sanitize_file_name("...", "Chapter"), "Chapter");
    }

    #[test]
    fn sanitize_caps_characters_and_bytes() {
        let long = "a".repeat(200);
        assert_eq!(sanitize_file_name(&long, "x").chars().count(), 80);
        // 4-byte characters: the byte cap (120) bites before the character cap.
        let wide = "😀".repeat(80);
        let out = sanitize_file_name(&wide, "x");
        assert!(out.len() <= 120);
        assert_eq!(out.chars().count(), 30);
    }

    #[test]
    fn chapter_file_names_are_recognised() {
        assert!(is_chapter_file_name("01 - Opening Credits.mp3"));
        assert!(is_chapter_file_name("100 - Epilogue.mp3"));
        assert!(!is_chapter_file_name("1 - Too short.mp3"));
        assert!(!is_chapter_file_name("01 - Book.m4b"));
        assert!(!is_chapter_file_name("cover.jpg"));
    }

    #[test]
    fn temp_paths_keep_the_extension_and_hide_the_file() {
        let temp = temp_path_for(Path::new("/books/A/Title.m4b"));
        assert_eq!(temp, Path::new("/books/A/.librisync-tmp.Title.m4b"));
    }

    #[test]
    fn free_space_check_passes_for_small_needs_and_fails_for_huge_ones() {
        let dir = std::env::temp_dir();
        if available_space(&dir).unwrap().is_some() {
            assert!(require_free_space(&dir, 1).is_ok());
            let err = require_free_space(&dir, u64::MAX / 2).unwrap_err();
            assert_eq!(err.kind, FailureKind::DiskFull);
        }
    }

    #[cfg(unix)]
    #[test]
    fn enospc_is_disk_full() {
        let e = io::Error::from_raw_os_error(libc::ENOSPC);
        assert_eq!(io_error("writing", e).kind, FailureKind::DiskFull);
        let e = io::Error::from(io::ErrorKind::PermissionDenied);
        assert_eq!(io_error("writing", e).kind, FailureKind::Io);
    }
}
