//! Live end-to-end test of `rust_core::pipeline` with a real DRM-free store (Chirp).
//!
//! Ignored by default: it needs a real Chirp account and downloads a whole book. Point
//! `CHIRP_CREDS_FILE` at a file holding `email:password` (or `email password`) on one
//! line — keep that file out of git:
//!
//! ```bash
//! CHIRP_CREDS_FILE=../../testing/chirp-creds.txt \
//!   cargo test --test pipeline_live_test -- --ignored --nocapture
//! ```
//!
//! Set `PIPELINE_LIVE_OUT` to a directory to keep the finished book there instead of in
//! a temp dir that is deleted afterwards, and `PIPELINE_LIVE_FORMAT=mp3` to test MP3
//! per chapter instead of M4B.
//!
//! Flow: sign in → sync the library into an in-memory DB → download plan for the first
//! book → `pipeline::run` → assert one chaptered, tagged M4B (or one tagged MP3 per
//! chapter).

use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rust_core::pipeline::{self, BookMeta, MediaTools, OutputFormat, PipelineJob, Progress, Stage};
use rust_core::providers::{AnyProvider, ProviderId};
use rust_core::storage::{queries, Database};
use tokio_util::sync::CancellationToken;

fn credentials() -> (String, String) {
    let path = std::env::var("CHIRP_CREDS_FILE")
        .expect("set CHIRP_CREDS_FILE to a file containing email:password");
    let line = std::fs::read_to_string(&path).expect("read CHIRP_CREDS_FILE");
    let line = line.trim();
    let split = line
        .find(|c: char| c == ':' || c.is_whitespace())
        .expect("expected email:password");
    let (email, rest) = line.split_at(split);
    (
        email.to_string(),
        rest[1..].trim_start_matches([':', ' ', '\t']).to_string(),
    )
}

fn split_names(value: Option<&str>) -> Vec<String> {
    value
        .map(|s| s.split(", ").map(str::to_string).filter(|n| !n.is_empty()).collect())
        .unwrap_or_default()
}

#[tokio::test]
#[ignore]
async fn chirp_book_through_the_pipeline() {
    let tools = MediaTools::from_env();
    let version = tools.check().await.expect("ffmpeg with libmp3lame and audible_key support");
    println!("ffmpeg {version}");

    let (email, password) = credentials();
    let any = AnyProvider::get(ProviderId::Chirp).unwrap();
    let chirp = any.as_provider();
    let creds = chirp
        .login(serde_json::json!({ "username": email, "password": password }))
        .await
        .expect("sign in");
    let account = serde_json::json!({
        "account_id": creds["account_id"],
        "account_name": email,
        "provider": "chirp",
        "identity": creds,
    });

    let db = Database::new_in_memory().await.unwrap();
    let mut page = 1;
    loop {
        let stats = chirp.sync_library_page(&db, &account, page).await.expect("sync page");
        if !stats.has_more {
            break;
        }
        page += 1;
        assert!(page < 100, "pagination never ended");
    }

    let product_id: String =
        sqlx::query_scalar("SELECT audible_product_id FROM Books WHERE source = 'chirp' LIMIT 1")
            .fetch_one(db.pool())
            .await
            .expect("the test account owns a book");
    let book = queries::find_book_with_relations_by_asin(db.pool(), &product_id)
        .await
        .unwrap()
        .expect("book row");
    let plan = chirp
        .download_plan(&db, &account, &product_id, &serde_json::json!({}))
        .await
        .expect("download plan");
    println!(
        "book {product_id} \"{}\": {} tracks, {} chapters",
        book.title,
        plan.parts.len(),
        plan.chapters.len()
    );
    let expected_chapters = plan.chapters.len();
    let expected_ms = plan.chapters.iter().map(|c| c.end_ms).max().unwrap_or(0);

    let meta = BookMeta {
        product_id: product_id.clone(),
        title: book.title.clone(),
        subtitle: book.subtitle.clone(),
        authors: split_names(book.authors_str.as_deref()),
        narrators: split_names(book.narrators_str.as_deref()),
        series: book.series_name.clone(),
        series_sequence: book.series_sequence.map(|s| {
            if s.fract() == 0.0 { format!("{s:.0}") } else { s.to_string() }
        }),
        publisher: book.publisher.clone(),
        release_date: book.date_published.clone(),
        description: Some(book.description.clone()).filter(|d| !d.is_empty()),
        language: book.language.clone(),
        genres: vec![],
        asin: None,
        isbn: None,
        cover_url: book.picture_large.clone(),
    };

    let temp = tempfile::tempdir().unwrap();
    let root = std::env::var_os("PIPELINE_LIVE_OUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| temp.path().to_path_buf());
    let format = match std::env::var("PIPELINE_LIVE_FORMAT").as_deref() {
        Ok("mp3") => OutputFormat::Mp3,
        _ => OutputFormat::M4b,
    };
    let job = PipelineJob {
        plan,
        meta,
        format,
        work_dir: root.join("work").join(&product_id),
        dest_dir: root.join("library").join(&product_id),
        file_stem: book.title.clone(),
        write_sidecars: true,
    };

    let started = Instant::now();
    let last: Arc<Mutex<(Option<Stage>, Instant)>> = Arc::new(Mutex::new((None, Instant::now())));
    let progress = Arc::new(move |p: Progress| {
        let mut last = last.lock().unwrap();
        if last.0 != Some(p.stage) || last.1.elapsed() >= Duration::from_secs(15) {
            println!(
                "[{:>5.0}s] {:?} part {}/{} {} / {:?} bytes, {:?} B/s, fraction {:?}",
                started.elapsed().as_secs_f32(),
                p.stage,
                p.part,
                p.parts,
                p.bytes_done,
                p.bytes_total,
                p.bytes_per_sec,
                p.fraction
            );
            *last = (Some(p.stage), Instant::now());
        }
    });
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    let out = pipeline::run(job, &tools, &http, progress, CancellationToken::new())
        .await
        .expect("pipeline");
    println!(
        "done in {:.0}s: {:?}, {} bytes, {:?} ms, {} chapters",
        started.elapsed().as_secs_f32(),
        out.audio_files,
        out.total_bytes,
        out.duration_ms,
        out.chapter_count
    );

    assert_eq!(out.chapter_count, expected_chapters);
    for sidecar in ["metadata.opf", "reader.txt"] {
        assert!(out.dest_dir.join(sidecar).exists(), "{sidecar} written");
    }
    let ffprobe = |file: &std::path::Path| -> serde_json::Value {
        let out = Command::new(&tools.ffprobe)
            .args(["-v", "error", "-print_format", "json", "-show_chapters", "-show_format", "-show_streams"])
            .arg(file)
            .output()
            .unwrap();
        serde_json::from_slice(&out.stdout).unwrap()
    };

    if format == OutputFormat::Mp3 {
        assert_eq!(out.audio_files.len(), expected_chapters);
        for (i, file) in out.audio_files.iter().enumerate() {
            let probe = ffprobe(file);
            assert_eq!(probe["streams"][0]["codec_name"], "mp3");
            assert_eq!(
                probe["format"]["tags"]["track"].as_str(),
                Some(format!("{}/{expected_chapters}", i + 1).as_str())
            );
            assert_eq!(probe["format"]["tags"]["album"].as_str(), Some(book.title.as_str()));
        }
        return;
    }

    assert_eq!(out.audio_files.len(), 1);
    let file = &out.audio_files[0];
    assert_eq!(file.extension().and_then(|e| e.to_str()), Some("m4b"));
    let duration = out.duration_ms.expect("duration");
    assert!(
        duration.abs_diff(expected_ms) < expected_ms / 100 + 2_000,
        "duration {duration} ms vs chapters ending at {expected_ms} ms"
    );

    let probe = ffprobe(file);
    assert_eq!(probe["chapters"].as_array().unwrap().len(), expected_chapters);
    assert_eq!(probe["format"]["tags"]["album"].as_str(), Some(book.title.as_str()));
}
