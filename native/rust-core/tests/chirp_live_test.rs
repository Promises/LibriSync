//! Live test of the Chirp provider against the real API.
//!
//! Ignored by default: it needs a real Chirp account. Point `CHIRP_CREDS_FILE` at a
//! file holding `email:password` (or `email password`) on one line — keep that file
//! out of git:
//!
//! ```bash
//! CHIRP_CREDS_FILE=../../testing/chirp-creds.txt \
//!   cargo test --test chirp_live_test -- --ignored --nocapture
//! ```
//!
//! Covers the whole provider surface: sign-in, a full paged library sync into a
//! fresh database, the download plan for the first book, and a ranged fetch of its
//! first track to prove the signed URL needs no extra headers.

use rust_core::providers::{AnyProvider, DownloadPart, ProviderId};
use rust_core::storage::{queries, Database};

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

#[tokio::test]
#[ignore]
async fn chirp_login_sync_and_plan() {
    let (email, password) = credentials();
    let any = AnyProvider::get(ProviderId::Chirp).unwrap();
    let chirp = any.as_provider();

    let creds = chirp
        .login(serde_json::json!({ "username": email, "password": password }))
        .await
        .expect("sign in");
    let account_id = creds["account_id"].as_str().unwrap().to_string();
    assert!(account_id.starts_with("chirp:"), "account id is namespaced");
    // What the bridge hands the provider: the saved account, not the bare blob.
    let account = serde_json::json!({
        "account_id": account_id,
        "account_name": email,
        "provider": "chirp",
        "identity": creds,
    });

    let db = Database::new_in_memory().await.unwrap();
    let mut page = 1;
    let mut synced = 0;
    loop {
        let stats = chirp
            .sync_library_page(&db, &account, page)
            .await
            .expect("sync page");
        println!(
            "page {page}: {} items, {} added, {} failed, errors {:?}",
            stats.total_items, stats.books_added, stats.items_failed, stats.errors
        );
        assert_eq!(stats.items_failed, 0);
        synced += stats.books_added;
        if !stats.has_more {
            break;
        }
        page += 1;
        assert!(page < 100, "pagination never ended");
    }
    assert!(synced > 0, "test account owns at least one book");

    // A second sync must not duplicate anything.
    let again = chirp.sync_library_page(&db, &account, 1).await.unwrap();
    assert_eq!(again.books_added, 0);

    let product_id: String =
        sqlx::query_scalar("SELECT audible_product_id FROM Books WHERE source = 'chirp' LIMIT 1")
            .fetch_one(db.pool())
            .await
            .unwrap();
    let book = queries::find_book_by_asin(db.pool(), &product_id)
        .await
        .unwrap()
        .unwrap();
    println!("first book: {product_id} {:?}", book.title);
    let owner: String = sqlx::query_scalar("SELECT account FROM LibraryBooks WHERE book_id = ?")
        .bind(book.book_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(owner, account_id, "books link to the namespaced account id");

    let plan = chirp
        .download_plan(&db, &account, &product_id, &serde_json::json!({}))
        .await
        .expect("download plan");
    assert!(plan.join_parts);
    assert_eq!(plan.parts.len(), plan.chapters.len());
    println!("plan: {} parts", plan.parts.len());

    let DownloadPart::Plain { url, headers, .. } = &plan.parts[0] else {
        panic!("Chirp plans are plain parts");
    };
    assert!(headers.is_empty());
    let resp = reqwest::Client::new()
        .get(url)
        .header("Range", "bytes=0-15")
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "track fetch: {}", resp.status());
    let head = resp.bytes().await.unwrap();
    let is_mp4 = head.len() >= 8 && &head[4..8] == b"ftyp";
    let is_mp3 = head.starts_with(b"ID3") || head.starts_with(&[0xFF, 0xFB]);
    assert!(is_mp4 || is_mp3, "first track is audio: {:02x?}", &head[..]);
}
