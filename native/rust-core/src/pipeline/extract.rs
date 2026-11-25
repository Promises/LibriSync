//! ZIP extraction for store-provided archives (Libro.fm MP3 parts, LibriVox).
//!
//! Port of `extractZipToDirectory` in `DownloadOrchestrator.kt`: only audio entries are
//! extracted, and each entry's directory prefix is dropped so the files land directly
//! in the book folder. Unlike Android, entries whose path would escape the archive
//! root are refused outright, and two entries with the same file name are both kept
//! (` (2)` suffix) instead of the second silently overwriting the first.

use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use super::fsutil::{io_error, require_free_space, sanitize_file_name, temp_path_for, AUDIO_EXTENSIONS};
use super::progress::Reporter;
use super::tools::cancelled;
use super::{FailureKind, PipelineError, Stage};

const COPY_BUFFER: usize = 256 * 1024;

/// The flattened, sanitised output name for an archive entry, or `None` for entries
/// that aren't audio or whose path is unsafe.
pub(crate) fn output_name(enclosed: Option<&Path>) -> Option<String> {
    let path = enclosed?;
    let file_name = path.file_name()?.to_str()?;
    let as_path = Path::new(file_name);
    let ext = as_path.extension()?.to_str()?.to_ascii_lowercase();
    if !AUDIO_EXTENSIONS.contains(&ext.as_str()) {
        return None;
    }
    let stem = as_path.file_stem()?.to_str()?;
    Some(format!("{}.{ext}", sanitize_file_name(stem, "track")))
}

/// `name`, or `name (2)`, `name (3)`, … — whichever is not yet in `taken`.
pub(crate) fn unique_name(name: &str, taken: &mut HashSet<String>) -> String {
    if taken.insert(name.to_string()) {
        return name.to_string();
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s, format!(".{e}")),
        None => (name, String::new()),
    };
    (2..)
        .map(|n| format!("{stem} ({n}){ext}"))
        .find(|candidate| taken.insert(candidate.clone()))
        .expect("an unused name exists")
}

/// Extract the audio entries of `zip_path` into `dest_dir` under temp names. Returns
/// `(temp, final)` pairs for the caller's staging; on error, every temp written so far
/// is removed. Blocking: run it on a blocking thread.
pub(crate) fn extract_audio(
    zip_path: &Path,
    dest_dir: &Path,
    taken: &mut HashSet<String>,
    reporter: &Arc<Reporter>,
    cancel: &CancellationToken,
) -> Result<Vec<(PathBuf, PathBuf)>, PipelineError> {
    let file = File::open(zip_path)
        .map_err(|e| io_error(format_args!("opening {}", zip_path.display()), e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| {
        PipelineError::new(
            FailureKind::Validation,
            format!("{} is not a readable ZIP archive: {e}", zip_path.display()),
        )
    })?;

    // Pass 1: decide what to extract and how much space it needs.
    let mut entries: Vec<(usize, String, u64)> = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|e| {
            PipelineError::new(FailureKind::Validation, format!("damaged ZIP entry {index}: {e}"))
        })?;
        if entry.is_dir() || entry.is_symlink() {
            continue;
        }
        let enclosed = entry.enclosed_name();
        if enclosed.is_none() {
            log::warn!(
                "pipeline: skipping ZIP entry with an unsafe path: {:?}",
                entry.name().unwrap_or_default()
            );
            continue;
        }
        if let Some(name) = output_name(enclosed.as_deref()) {
            entries.push((index, unique_name(&name, taken), entry.size()));
        }
    }
    if entries.is_empty() {
        return Err(PipelineError::new(
            FailureKind::Validation,
            format!("No audio files found in {}", zip_path.display()),
        ));
    }
    let total: u64 = entries.iter().map(|(_, _, size)| size).sum();
    require_free_space(dest_dir, total)?;

    // Pass 2: extract.
    let mut written: Vec<(PathBuf, PathBuf)> = Vec::new();
    let result = (|| {
        let mut done = 0u64;
        let mut buffer = vec![0u8; COPY_BUFFER];
        for (index, name, _) in &entries {
            let final_path = dest_dir.join(name);
            let temp = temp_path_for(&final_path);
            written.push((temp.clone(), final_path));
            let mut entry = archive.by_index(*index).map_err(|e| {
                PipelineError::new(FailureKind::Validation, format!("damaged ZIP entry {name}: {e}"))
            })?;
            let mut out = File::create(&temp)
                .map_err(|e| io_error(format_args!("creating {}", temp.display()), e))?;
            loop {
                if cancel.is_cancelled() {
                    return Err(cancelled());
                }
                let n = entry.read(&mut buffer).map_err(|e| {
                    PipelineError::new(
                        FailureKind::Validation,
                        format!("could not extract {name}: {e}"),
                    )
                })?;
                if n == 0 {
                    break;
                }
                out.write_all(&buffer[..n])
                    .map_err(|e| io_error(format_args!("writing {}", temp.display()), e))?;
                done += n as u64;
                if total > 0 {
                    reporter.fraction(Stage::Extracting, done as f32 / total as f32);
                }
            }
            out.flush()
                .map_err(|e| io_error(format_args!("writing {}", temp.display()), e))?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => Ok(written),
        Err(e) => {
            for (temp, _) in &written {
                let _ = std::fs::remove_file(temp);
            }
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_names_flatten_and_filter() {
        assert_eq!(output_name(Some(Path::new("CD1/01 Intro.mp3"))).as_deref(), Some("01 Intro.mp3"));
        assert_eq!(output_name(Some(Path::new("Track.MP3"))).as_deref(), Some("Track.mp3"));
        assert_eq!(output_name(Some(Path::new("notes/readme.txt"))), None);
        assert_eq!(output_name(Some(Path::new("cover.jpg"))), None);
        assert_eq!(output_name(None), None);
        assert_eq!(output_name(Some(Path::new("a/Bad: name?.m4a"))).as_deref(), Some("Bad name.m4a"));
    }

    #[test]
    fn duplicate_names_get_a_suffix() {
        let mut taken = HashSet::new();
        assert_eq!(unique_name("01.mp3", &mut taken), "01.mp3");
        assert_eq!(unique_name("01.mp3", &mut taken), "01 (2).mp3");
        assert_eq!(unique_name("01.mp3", &mut taken), "01 (3).mp3");
        assert_eq!(unique_name("02.mp3", &mut taken), "02.mp3");
    }
}
