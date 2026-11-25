//! Integration tests for `rust_core::pipeline` against the system ffmpeg.
//!
//! Each test generates short sine-wave tracks with `ffmpeg -f lavfi`, serves them from a
//! tiny HTTP server on localhost (Range support, plus modes that ignore Range, trickle
//! the body slowly, or refuse with 403), and runs the pipeline into a temp dir.
//!
//! If ffmpeg/ffprobe (with libmp3lame) aren't installed, every test prints a note and
//! passes without running.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rust_core::pipeline::{
    self, BookMeta, FailureKind, MediaTools, OutputFormat, PipelineError, PipelineJob,
    PipelineOutput, Progress, ProgressFn,
};
use rust_core::providers::{DownloadPart, DownloadPlan, PlanChapter};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

// ------------------------------------------------------------------ test server

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Honour `Range`.
    Normal,
    /// Always answer 200 with the whole file.
    IgnoreRange,
    /// Honour `Range`, but trickle the body (8 KiB every 40 ms).
    Slow,
    /// Honour `Range` but ignore `If-Range`, so a changed file is spliced on unless the
    /// client notices.
    IgnoreIfRange,
}

#[derive(Debug, Clone)]
struct Hit {
    path: String,
    range: Option<String>,
    if_range: Option<String>,
    status: u16,
}

/// A strong ETag that changes with the content.
fn etag_of(bytes: &[u8]) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    format!("\"{:016x}-{}\"", hasher.finish(), bytes.len())
}

#[derive(Clone)]
struct TestServer {
    base: String,
    files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    mode: Arc<Mutex<Mode>>,
    hits: Arc<Mutex<Vec<Hit>>>,
}

impl TestServer {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = TestServer {
            base: format!("http://{addr}"),
            files: Arc::default(),
            mode: Arc::new(Mutex::new(Mode::Normal)),
            hits: Arc::default(),
        };
        let handle = server.clone();
        tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else { break };
                let handle = handle.clone();
                tokio::spawn(async move {
                    let _ = handle.serve(socket).await;
                });
            }
        });
        server
    }

    fn add(&self, name: &str, bytes: Vec<u8>) -> String {
        self.files.lock().unwrap().insert(format!("/{name}"), bytes);
        format!("{}/{name}", self.base)
    }

    fn set_mode(&self, mode: Mode) {
        *self.mode.lock().unwrap() = mode;
    }

    fn hits(&self) -> Vec<Hit> {
        self.hits.lock().unwrap().clone()
    }

    async fn serve(&self, mut socket: TcpStream) -> std::io::Result<()> {
        let mut request = Vec::new();
        let mut buf = [0u8; 4096];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = socket.read(&mut buf).await?;
            if n == 0 {
                return Ok(());
            }
            request.extend_from_slice(&buf[..n]);
        }
        let text = String::from_utf8_lossy(&request).into_owned();
        let mut lines = text.lines();
        let path = lines
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("/")
            .to_string();
        let headers: Vec<(String, String)> = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        let header = |name: &str| headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
        let range = header("range");
        let if_range = header("if-range");
        let mode = *self.mode.lock().unwrap();
        let file = self.files.lock().unwrap().get(&path).cloned();

        let record = |status: u16| {
            self.hits.lock().unwrap().push(Hit {
                path: path.clone(),
                range: range.clone(),
                if_range: if_range.clone(),
                status,
            });
        };
        let Some(file) = file else {
            let status = if path.starts_with("/expired") { 403 } else { 404 };
            record(status);
            let head = format!("HTTP/1.1 {status} Nope\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            socket.write_all(head.as_bytes()).await?;
            return Ok(());
        };
        let len = file.len() as u64;
        let etag = etag_of(&file);
        // RFC 9110 §13.1.5: a stale If-Range validator means "send the whole file".
        let if_range_holds = mode == Mode::IgnoreIfRange
            || if_range.as_deref().map_or(true, |v| v == etag);
        let start = match (&range, mode) {
            _ if !if_range_holds => None,
            (Some(r), Mode::Normal | Mode::Slow | Mode::IgnoreIfRange) => r
                .strip_prefix("bytes=")
                .and_then(|r| r.strip_suffix('-'))
                .and_then(|n| n.parse::<u64>().ok()),
            _ => None,
        };
        let (status, head, body) = match start {
            Some(s) if s >= len => {
                (416, format!("HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{len}\r\nContent-Length: 0\r\n"), &file[..0])
            }
            Some(s) => (
                206,
                format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {s}-{}/{len}\r\nContent-Length: {}\r\nETag: {etag}\r\n",
                    len - 1,
                    len - s
                ),
                &file[s as usize..],
            ),
            None => (200, format!("HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nETag: {etag}\r\n"), &file[..]),
        };
        record(status);
        socket.write_all(format!("{head}Connection: close\r\n\r\n").as_bytes()).await?;
        if mode == Mode::Slow {
            for chunk in body.chunks(8 * 1024) {
                socket.write_all(chunk).await?;
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
        } else {
            socket.write_all(body).await?;
        }
        socket.shutdown().await?;
        Ok(())
    }
}

// ------------------------------------------------------------------ helpers

async fn tools_or_skip() -> Option<MediaTools> {
    let tools = MediaTools::from_env();
    match tools.check().await {
        Ok(version) => {
            println!("using ffmpeg {version}");
            Some(tools)
        }
        Err(e) => {
            println!("NOTE: skipping pipeline integration test — {}", e.message);
            None
        }
    }
}

fn ffmpeg(args: &[&str]) {
    let status = Command::new(std::env::var("FFMPEG_PATH").unwrap_or_else(|_| "ffmpeg".into()))
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .status()
        .expect("run ffmpeg");
    assert!(status.success(), "ffmpeg {args:?}");
}

/// A sine tone. `encoder` is `aac` or `libmp3lame`.
fn tone(dir: &Path, name: &str, freq: u32, secs: u32, rate: u32, encoder: &str, extra: &[&str]) -> Vec<u8> {
    let path = dir.join(name);
    let source = format!("sine=frequency={freq}:duration={secs}:sample_rate={rate}");
    let mut args = vec!["-f", "lavfi", "-i", &source, "-c:a", encoder, "-b:a", "96k"];
    args.extend_from_slice(extra);
    args.push(path.to_str().unwrap());
    ffmpeg(&args);
    std::fs::read(path).unwrap()
}

fn cover_jpeg(dir: &Path) -> Vec<u8> {
    let path = dir.join("cover-src.jpg");
    ffmpeg(&["-f", "lavfi", "-i", "color=c=blue:s=64x64:d=1", "-frames:v", "1", path.to_str().unwrap()]);
    std::fs::read(path).unwrap()
}

fn ffprobe(path: &Path) -> Value {
    let out = Command::new(std::env::var("FFPROBE_PATH").unwrap_or_else(|_| "ffprobe".into()))
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams", "-show_chapters"])
        .arg(path)
        .output()
        .expect("run ffprobe");
    assert!(out.status.success(), "ffprobe {}", path.display());
    serde_json::from_slice(&out.stdout).unwrap()
}

fn tag<'a>(probe: &'a Value, key: &str) -> Option<&'a str> {
    probe["format"]["tags"][key].as_str()
}

fn has_cover(probe: &Value) -> bool {
    probe["streams"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] == 1)
}

fn meta(cover_url: Option<String>) -> BookMeta {
    BookMeta {
        product_id: "chirp_123".into(),
        title: "The Test Book".into(),
        subtitle: None,
        authors: vec!["Ann Author".into()],
        narrators: vec!["Nora Narrator".into(), "Ned Reader".into()],
        series: Some("Test Saga".into()),
        series_sequence: Some("2".into()),
        publisher: Some("Test House".into()),
        release_date: Some("2021-03-04".into()),
        description: Some("A book used in tests.".into()),
        language: Some("English".into()),
        genres: vec!["Fiction".into()],
        asin: None,
        isbn: Some("9780000000002".into()),
        cover_url,
    }
}

fn plain(url: String, filename: &str) -> DownloadPart {
    DownloadPart::Plain { url, headers: Default::default(), filename: filename.into() }
}

fn job(plan: DownloadPlan, meta: BookMeta, format: OutputFormat, root: &Path, sidecars: bool) -> PipelineJob {
    PipelineJob {
        plan,
        meta,
        format,
        work_dir: root.join("work"),
        dest_dir: root.join("library/Ann Author/The Test Book"),
        file_stem: "The Test Book".into(),
        write_sidecars: sidecars,
    }
}

fn collect_progress() -> (ProgressFn, Arc<Mutex<Vec<Progress>>>) {
    let seen: Arc<Mutex<Vec<Progress>>> = Arc::default();
    let sink = Arc::clone(&seen);
    (Arc::new(move |p: Progress| sink.lock().unwrap().push(p)), seen)
}

async fn run(job: PipelineJob, tools: &MediaTools) -> Result<PipelineOutput, PipelineError> {
    let (progress, _) = collect_progress();
    pipeline::run(job, tools, &reqwest::Client::new(), progress, CancellationToken::new()).await
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn three_track_plan(server: &TestServer, scratch: &Path) -> DownloadPlan {
    let parts = (1..=3)
        .map(|i| {
            let bytes = tone(scratch, &format!("src{i}.m4a"), 300 * i, 2, 44_100, "aac", &[]);
            plain(server.add(&format!("track{i}.m4a"), bytes), &format!("{i:03}.m4a"))
        })
        .collect();
    let chapters = ["One", "Two: The Sequel", "Three"]
        .iter()
        .enumerate()
        .map(|(i, title)| PlanChapter {
            title: title.to_string(),
            start_ms: i as u64 * 2000,
            end_ms: (i as u64 + 1) * 2000,
        })
        .collect();
    DownloadPlan { parts, embed_metadata: true, chapters, join_parts: true, licence_throttled: false }
}

// ------------------------------------------------------------------ tests

#[tokio::test]
async fn check_reports_a_version() {
    let Some(tools) = tools_or_skip().await else { return };
    let version = tools.check().await.unwrap();
    assert!(!version.is_empty());

    let missing = MediaTools { ffmpeg: "/nonexistent/ffmpeg".into(), ffprobe: tools.ffprobe.clone() };
    let err = missing.check().await.unwrap_err();
    assert_eq!(err.kind, FailureKind::Ffmpeg);
}

#[tokio::test]
async fn single_plain_m4b_is_tagged_with_cover_and_sidecars() {
    let Some(tools) = tools_or_skip().await else { return };
    let root = tempfile::tempdir().unwrap();
    let server = TestServer::start().await;
    let book = tone(root.path(), "book-src.m4b", 440, 4, 44_100, "aac", &["-f", "ipod"]);
    let url = server.add("book.m4b", book);
    let cover = server.add("cover.jpg", cover_jpeg(root.path()));

    let plan = DownloadPlan {
        parts: vec![plain(url, "B0TEST.m4b")],
        embed_metadata: true,
        chapters: vec![],
        join_parts: false,
        licence_throttled: false,
    };
    let job = job(plan, meta(Some(cover)), OutputFormat::M4b, root.path(), true);
    let (dest, work) = (job.dest_dir.clone(), job.work_dir.clone());
    let out = run(job, &tools).await.expect("pipeline");

    assert_eq!(out.audio_files, vec![dest.join("The Test Book.m4b")]);
    assert!(out.total_bytes > 0);
    let duration = out.duration_ms.unwrap();
    assert!((3900..=4200).contains(&duration), "duration {duration}");
    assert_eq!(
        names_in(&dest),
        vec!["The Test Book.m4b", "cover.jpg", "desc.txt", "metadata.opf", "reader.txt"]
    );
    assert!(!work.exists(), "work dir removed on success");

    let probe = ffprobe(&out.audio_files[0]);
    assert_eq!(tag(&probe, "title"), Some("The Test Book"));
    assert_eq!(tag(&probe, "album"), Some("The Test Book"));
    assert_eq!(tag(&probe, "artist"), Some("Ann Author"));
    assert_eq!(tag(&probe, "composer"), Some("Nora Narrator, Ned Reader"));
    assert!(has_cover(&probe), "cover embedded");
    assert_eq!(std::fs::read_to_string(dest.join("reader.txt")).unwrap(), "Nora Narrator, Ned Reader\n");
    let opf = std::fs::read_to_string(dest.join("metadata.opf")).unwrap();
    assert!(opf.contains("<dc:identifier id=\"BookId\" opf:scheme=\"ISBN\">9780000000002</dc:identifier>"));
}

#[tokio::test]
async fn joined_tracks_become_one_chaptered_m4b() {
    let Some(tools) = tools_or_skip().await else { return };
    let root = tempfile::tempdir().unwrap();
    let server = TestServer::start().await;
    let plan = three_track_plan(&server, root.path());
    let cover = server.add("cover.jpg", cover_jpeg(root.path()));
    let job = job(plan, meta(Some(cover)), OutputFormat::M4b, root.path(), false);
    let dest = job.dest_dir.clone();
    let (progress, seen) = collect_progress();
    let out = pipeline::run(job, &tools, &reqwest::Client::new(), progress, CancellationToken::new())
        .await
        .expect("pipeline");

    assert_eq!(out.chapter_count, 3);
    assert_eq!(names_in(&dest), vec!["The Test Book.m4b"]);
    let probe = ffprobe(&out.audio_files[0]);
    let chapters = probe["chapters"].as_array().unwrap();
    let titles: Vec<&str> = chapters.iter().map(|c| c["tags"]["title"].as_str().unwrap()).collect();
    assert_eq!(titles, vec!["One", "Two: The Sequel", "Three"]);
    assert_eq!(probe["streams"][0]["codec_name"], "aac");
    assert_eq!(tag(&probe, "title"), Some("The Test Book"));
    assert_eq!(tag(&probe, "album"), Some("The Test Book"));
    assert_eq!(tag(&probe, "album_artist"), Some("Ann Author"));
    assert_eq!(tag(&probe, "show"), Some("Test Saga"));
    assert_eq!(tag(&probe, "episode_id"), Some("2"));
    assert_eq!(tag(&probe, "date"), Some("2021"));
    assert_eq!(tag(&probe, "genre"), Some("Fiction"));
    assert!(has_cover(&probe));

    let stages: Vec<_> = seen.lock().unwrap().iter().map(|p| p.stage).collect();
    for stage in [pipeline::Stage::Downloading, pipeline::Stage::Joining, pipeline::Stage::Validating] {
        assert!(stages.contains(&stage), "{stage:?} reported");
    }
}

#[tokio::test]
async fn joined_tracks_as_mp3_per_chapter_replace_a_previous_m4b() {
    let Some(tools) = tools_or_skip().await else { return };
    let root = tempfile::tempdir().unwrap();
    let server = TestServer::start().await;
    let plan = three_track_plan(&server, root.path());

    // First an M4B, then the same book again as MP3 into the same folder.
    let first = run(job(plan.clone(), meta(None), OutputFormat::M4b, root.path(), false), &tools)
        .await
        .expect("m4b run");
    assert!(first.audio_files[0].exists());

    let job = job(plan, meta(None), OutputFormat::Mp3, root.path(), false);
    let dest = job.dest_dir.clone();
    let out = run(job, &tools).await.expect("mp3 run");
    assert_eq!(out.chapter_count, 3);
    assert_eq!(
        names_in(&dest),
        vec!["01 - One.mp3", "02 - Two The Sequel.mp3", "03 - Three.mp3"],
        "previous M4B removed"
    );
    for (i, (file, title)) in out
        .audio_files
        .iter()
        .zip(["One", "Two: The Sequel", "Three"])
        .enumerate()
    {
        let probe = ffprobe(file);
        assert_eq!(probe["streams"][0]["codec_name"], "mp3");
        assert_eq!(tag(&probe, "title"), Some(title));
        assert_eq!(tag(&probe, "track"), Some(format!("{}/3", i + 1).as_str()));
        assert_eq!(tag(&probe, "album"), Some("The Test Book"));
        assert_eq!(tag(&probe, "artist"), Some("Ann Author"));
        assert_eq!(tag(&probe, "series"), Some("Test Saga"));
        assert!(probe["chapters"].as_array().unwrap().is_empty(), "no per-file chapters");
        let secs: f64 = probe["format"]["duration"].as_str().unwrap().parse().unwrap();
        assert!((1.8..2.3).contains(&secs), "chapter {} lasts {secs}s", i + 1);
    }
}

#[tokio::test]
async fn mixed_codec_tracks_are_joined_by_reencoding() {
    let Some(tools) = tools_or_skip().await else { return };
    let root = tempfile::tempdir().unwrap();
    let server = TestServer::start().await;
    let a = tone(root.path(), "a.m4a", 300, 2, 44_100, "aac", &[]);
    let b = tone(root.path(), "b.mp3", 600, 2, 22_050, "libmp3lame", &["-ac", "1"]);
    let plan = DownloadPlan {
        parts: vec![
            plain(server.add("a.m4a", a), "001.m4a"),
            plain(server.add("b.mp3", b), "002.mp3"),
        ],
        embed_metadata: true,
        chapters: vec![],
        join_parts: true,
        licence_throttled: false,
    };
    let out = run(job(plan, meta(None), OutputFormat::M4b, root.path(), false), &tools)
        .await
        .expect("pipeline");
    let probe = ffprobe(&out.audio_files[0]);
    assert_eq!(probe["streams"][0]["codec_name"], "aac");
    // No plan chapters: one per part.
    let titles: Vec<&str> = probe["chapters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["tags"]["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, vec!["Part 1", "Part 2"]);
}

fn store_zip() -> Vec<u8> {
    use std::io::Write;
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, body) in [
            ("Book/CD1/01 - Intro.mp3", b"ID3-first".as_slice()),
            ("Book/02 - Next.mp3", b"ID3-second".as_slice()),
            ("Book/readme.txt", b"not audio".as_slice()),
            ("../escape.mp3", b"evil".as_slice()),
        ] {
            zip.start_file(name, options).unwrap();
            zip.write_all(body).unwrap();
        }
        zip.finish().unwrap();
    }
    cursor.into_inner()
}

#[tokio::test]
async fn zip_parts_are_extracted_flat_and_safely() {
    let Some(tools) = tools_or_skip().await else { return };
    let root = tempfile::tempdir().unwrap();
    let server = TestServer::start().await;
    let url = server.add("parts.zip", store_zip());
    let plan = DownloadPlan {
        parts: vec![DownloadPart::Zip { url, headers: Default::default() }],
        embed_metadata: false,
        chapters: vec![],
        join_parts: false,
        licence_throttled: false,
    };
    let job = job(plan, meta(None), OutputFormat::Mp3, root.path(), true);
    let dest = job.dest_dir.clone();
    let out = run(job, &tools).await.expect("pipeline");

    assert_eq!(out.audio_files, vec![dest.join("01 - Intro.mp3"), dest.join("02 - Next.mp3")]);
    assert_eq!(std::fs::read(dest.join("01 - Intro.mp3")).unwrap(), b"ID3-first");
    assert_eq!(
        names_in(&dest),
        vec!["01 - Intro.mp3", "02 - Next.mp3", "desc.txt", "metadata.opf", "reader.txt"]
    );
    assert!(!dest.parent().unwrap().join("escape.mp3").exists());
    assert!(!root.path().join("escape.mp3").exists());
}

/// Start a run against a slow server and cancel it once some bytes are on disk.
/// Returns the partial file's size.
async fn cancelled_partial_download(
    tools: &MediaTools,
    server: &TestServer,
    plan: DownloadPlan,
    root: &Path,
) -> (u64, PathBuf) {
    server.set_mode(Mode::Slow);
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let progress: ProgressFn = Arc::new(move |p: Progress| {
        if p.bytes_done > 64 * 1024 {
            trigger.cancel();
        }
    });
    let job = job(plan, meta(None), OutputFormat::M4b, root, false);
    let work = job.work_dir.clone();
    let err = pipeline::run(job, tools, &reqwest::Client::new(), progress, cancel)
        .await
        .expect_err("cancelled");
    assert_eq!(err.kind, FailureKind::Cancelled);
    let partial = work.join("book.m4b.part");
    let size = std::fs::metadata(&partial).expect("partial kept").len();
    (size, partial)
}

fn big_plain_plan(server: &TestServer, scratch: &Path) -> (DownloadPlan, Vec<u8>) {
    let bytes = tone(scratch, "big-src.m4b", 440, 40, 44_100, "aac", &["-f", "ipod"]);
    let url = server.add("big.m4b", bytes.clone());
    let plan = DownloadPlan {
        parts: vec![plain(url, "book.m4b")],
        embed_metadata: false,
        chapters: vec![],
        join_parts: false,
        licence_throttled: false,
    };
    (plan, bytes)
}

#[tokio::test]
async fn cancelled_download_resumes_with_a_range_request() {
    let Some(tools) = tools_or_skip().await else { return };
    let root = tempfile::tempdir().unwrap();
    let server = TestServer::start().await;
    let (plan, original) = big_plain_plan(&server, root.path());
    let (partial_size, _) = cancelled_partial_download(&tools, &server, plan.clone(), root.path()).await;
    assert!(partial_size > 0 && partial_size < original.len() as u64);

    server.set_mode(Mode::Normal);
    let out = run(job(plan, meta(None), OutputFormat::M4b, root.path(), false), &tools)
        .await
        .expect("resumed run");
    let resumed = server
        .hits()
        .into_iter()
        .find(|h| h.range.is_some())
        .expect("a ranged request");
    assert_eq!(resumed.path, "/big.m4b");
    assert_eq!(resumed.range.as_deref(), Some(format!("bytes={partial_size}-").as_str()));
    assert!(resumed.if_range.is_some(), "the resume names the object it continues");
    assert_eq!(resumed.status, 206);
    assert_eq!(std::fs::read(&out.audio_files[0]).unwrap(), original, "byte-identical");
}

#[tokio::test]
async fn a_server_that_ignores_range_restarts_the_download() {
    let Some(tools) = tools_or_skip().await else { return };
    let root = tempfile::tempdir().unwrap();
    let server = TestServer::start().await;
    let (plan, original) = big_plain_plan(&server, root.path());
    cancelled_partial_download(&tools, &server, plan.clone(), root.path()).await;

    server.set_mode(Mode::IgnoreRange);
    let out = run(job(plan, meta(None), OutputFormat::M4b, root.path(), false), &tools)
        .await
        .expect("restarted run");
    let last = server.hits().last().cloned().unwrap();
    assert!(last.range.is_some(), "the client asked to resume");
    assert_eq!(last.status, 200, "the server sent everything");
    // Appending the full body to the partial would corrupt it; restarting keeps it exact.
    assert_eq!(std::fs::read(&out.audio_files[0]).unwrap(), original);
}

/// The object behind the download's name is replaced between runs (a new signed URL for
/// a re-encoded file, a CDN swap). The partial of the old object must not be resumed.
async fn replace_object_behind_partial(mode: Mode) -> Option<(Vec<Hit>, Vec<u8>, Vec<u8>)> {
    let tools = tools_or_skip().await?;
    let root = tempfile::tempdir().unwrap();
    let server = TestServer::start().await;
    let (plan, _) = big_plain_plan(&server, root.path());
    let (partial_size, partial) =
        cancelled_partial_download(&tools, &server, plan.clone(), root.path()).await;
    assert!(partial_size > 0);
    assert!(
        partial.with_file_name("book.m4b.part.meta").exists(),
        "the partial records which object it holds"
    );
    let first_run_hits = server.hits().len();

    let other = tone(root.path(), "other-src.m4b", 880, 30, 44_100, "aac", &["-f", "ipod"]);
    server.add("big.m4b", other.clone());
    server.set_mode(mode);
    let out = run(job(plan, meta(None), OutputFormat::M4b, root.path(), false), &tools)
        .await
        .expect("run against the replaced object");
    let produced = std::fs::read(&out.audio_files[0]).unwrap();
    let hits = server.hits().split_off(first_run_hits);
    Some((hits, produced, other))
}

#[tokio::test]
async fn a_partial_of_a_replaced_object_is_not_resumed() {
    let Some((hits, produced, other)) = replace_object_behind_partial(Mode::Normal).await else {
        return;
    };
    let resume = &hits[0];
    assert!(resume.range.is_some() && resume.if_range.is_some(), "{resume:?}");
    assert_eq!(resume.status, 200, "a stale If-Range gets the whole new file");
    assert_eq!(produced, other, "no bytes of the old object survive");
}

#[tokio::test]
async fn a_server_ignoring_if_range_cannot_splice_two_objects() {
    let Some((hits, produced, other)) = replace_object_behind_partial(Mode::IgnoreIfRange).await
    else {
        return;
    };
    assert_eq!(hits[0].status, 206, "the server resumed regardless: {hits:?}");
    let restart = hits.iter().skip(1).find(|h| h.range.is_none()).expect("a fresh download");
    assert_eq!(restart.status, 200);
    assert_eq!(produced, other, "the changed object was noticed and fetched whole");
}

#[tokio::test]
async fn a_truncated_file_fails_validation_and_is_kept() {
    let Some(tools) = tools_or_skip().await else { return };
    let root = tempfile::tempdir().unwrap();
    let server = TestServer::start().await;
    let full = tone(root.path(), "full.m4b", 440, 20, 44_100, "aac", &["-movflags", "+faststart", "-f", "ipod"]);
    let truncated = full[..full.len() / 2].to_vec();
    let url = server.add("truncated.m4b", truncated.clone());
    let plan = DownloadPlan {
        parts: vec![plain(url, "book.m4b")],
        embed_metadata: false,
        chapters: vec![],
        join_parts: false,
        licence_throttled: false,
    };
    let job = job(plan, meta(None), OutputFormat::M4b, root.path(), false);
    let (work, dest) = (job.work_dir.clone(), job.dest_dir.clone());
    let err = run(job, &tools).await.expect_err("validation must fail");
    println!("validation error: {}", err.message);
    assert_eq!(err.kind, FailureKind::Validation, "{}", err.message);
    assert!(err.is_retryable());
    assert_eq!(std::fs::read(work.join("book.m4b")).unwrap(), truncated, "source kept for the caller");
    assert!(names_in(&dest).is_empty(), "nothing left in the book folder");
}

#[tokio::test]
async fn a_refused_link_is_reported_as_expired() {
    let Some(tools) = tools_or_skip().await else { return };
    let root = tempfile::tempdir().unwrap();
    let server = TestServer::start().await;
    let plan = DownloadPlan {
        parts: vec![plain(format!("{}/expired/book.m4b", server.base), "book.m4b")],
        embed_metadata: false,
        chapters: vec![],
        join_parts: false,
        licence_throttled: false,
    };
    let err = run(job(plan, meta(None), OutputFormat::M4b, root.path(), false), &tools)
        .await
        .expect_err("403");
    assert_eq!(err.kind, FailureKind::Expired);
}
