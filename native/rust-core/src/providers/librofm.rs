//! Libro.fm provider — a DRM-free audiobook store.
//!
//! Auth is a plain password grant (`POST /oauth/token`) → bearer token; the library
//! and downloads are simple JSON APIs (v10). No DRM, no device registration. Shapes
//! confirmed against the maintained reference tool (burntcookie90/librofm-downloader)
//! and verified against the live API.

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::Client;
use serde::{Deserialize, Deserializer};

use crate::api::library::SyncStats;
use crate::storage::{queries, Database};
use crate::{LibationError, Result};

use super::http;
use super::{
    CredentialBlob, DownloadPart, DownloadPlan, LoginInput, PlanChapter, PlanOptions, Provider,
    ProviderId,
};

const BASE: &str = "https://libro.fm";

/// Libro.fm's edge rejects anything that doesn't look like their own mobile app:
/// a browser User-Agent gets a bodyless `401` straight from the load balancer,
/// indistinguishable from bad credentials. These two headers are what the app
/// sends; bump them if the store starts rejecting this version.
const APP_UA: &str = "okhttp/5.3.2";
const APP_VERSION: &str = "7.34.8";

pub struct LibrofmProvider;

/// Libro.fm returns `isbn` as a JSON number; accept a string too, defensively.
fn isbn_string<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<String, D::Error> {
    match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(s) => Ok(s),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        other => Err(serde::de::Error::custom(format!(
            "unexpected isbn value: {other}"
        ))),
    }
}

/// Cover URLs come back protocol-relative (`//covers.libro.fm/…`).
fn absolute_cover_url(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        url.to_string()
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
}

#[derive(Deserialize)]
struct LibroLibrary {
    total_pages: i32,
    #[serde(default)]
    audiobooks: Vec<LibroBook>,
}

#[derive(Deserialize)]
struct LibroBook {
    title: String,
    #[serde(default)]
    authors: Vec<String>,
    #[serde(deserialize_with = "isbn_string")]
    isbn: String,
    #[serde(default)]
    cover_url: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    audiobook_info: Option<AudiobookInfo>,
}

#[derive(Deserialize)]
struct AudiobookInfo {
    #[serde(default)]
    narrators: Vec<String>,
    #[serde(default)]
    duration: i32, // seconds
}

#[derive(Deserialize)]
struct M4bMeta {
    m4b_url: String,
}

#[derive(Deserialize)]
struct DownloadManifest {
    #[serde(default)]
    parts: Vec<ManifestPart>,
    #[serde(default)]
    tracks: Vec<ManifestTrack>,
}

#[derive(Deserialize)]
struct ManifestPart {
    url: String,
}

#[derive(Deserialize)]
struct ManifestTrack {
    #[serde(default)]
    length_msec: u64,
    #[serde(default)]
    chapter_title: Option<String>,
    #[serde(default)]
    number: i32,
}

impl LibrofmProvider {
    /// A client that impersonates the Libro.fm mobile app (see [`APP_UA`]).
    fn client() -> Result<Client> {
        let mut headers = HeaderMap::new();
        headers.insert("X-LibroFm-AppVer", HeaderValue::from_static(APP_VERSION));
        http::build(http::builder(APP_UA).default_headers(headers))
    }

    /// Look up the bearer token whether `creds` is the raw credential blob
    /// (`{access_token,..}`) or a full account (`{identity:{access_token,..}}`).
    fn token_of(creds: &CredentialBlob) -> Result<String> {
        creds
            .get("access_token")
            .and_then(|v| v.as_str())
            .or_else(|| {
                creds
                    .get("identity")
                    .and_then(|i| i.get("access_token"))
                    .and_then(|v| v.as_str())
            })
            .map(String::from)
            .ok_or_else(|| LibationError::AuthenticationFailed {
                message: "Missing Libro.fm access token".into(),
                account_id: None,
            })
    }

    fn account_of(creds: &CredentialBlob) -> String {
        creds
            .get("username")
            .and_then(|v| v.as_str())
            .or_else(|| creds.get("account_id").and_then(|v| v.as_str()))
            .or_else(|| {
                creds
                    .get("identity")
                    .and_then(|i| i.get("username"))
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("librofm")
            .to_string()
    }

    /// Chapter markers from the manifest's `tracks[]`, which carry per-track
    /// durations rather than absolute offsets.
    fn chapters_from(tracks: &[ManifestTrack]) -> Vec<PlanChapter> {
        let mut cursor = 0u64;
        tracks
            .iter()
            .map(|t| {
                let start = cursor;
                cursor += t.length_msec;
                PlanChapter {
                    title: t
                        .chapter_title
                        .clone()
                        .unwrap_or_else(|| format!("Track {}", t.number)),
                    start_ms: start,
                    end_ms: cursor,
                }
            })
            .collect()
    }
}

#[async_trait]
impl Provider for LibrofmProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Librofm
    }

    async fn login(&self, input: LoginInput) -> Result<CredentialBlob> {
        let username = input
            .get("username")
            .and_then(|v| v.as_str())
            .ok_or_else(|| LibationError::InvalidInput("username required".into()))?;
        let password = input
            .get("password")
            .and_then(|v| v.as_str())
            .ok_or_else(|| LibationError::InvalidInput("password required".into()))?;

        let resp = Self::client()?
            .post(format!("{BASE}/oauth/token"))
            .json(&serde_json::json!({
                "grant_type": "password", "username": username, "password": password
            }))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(LibationError::AuthenticationFailed {
                message: format!("Libro.fm login failed ({})", resp.status()),
                account_id: Some(username.to_string()),
            });
        }
        let token: TokenResponse = resp.json().await?;
        let access = token
            .access_token
            .ok_or_else(|| LibationError::AuthenticationFailed {
                message: "Libro.fm returned no access token".into(),
                account_id: Some(username.to_string()),
            })?;
        Ok(serde_json::json!({ "access_token": access, "username": username }))
    }

    async fn refresh(&self, _creds: &CredentialBlob) -> Result<Option<CredentialBlob>> {
        Ok(None)
    }

    async fn sync_library_page(
        &self,
        db: &Database,
        creds: &CredentialBlob,
        page: i32,
    ) -> Result<SyncStats> {
        let token = Self::token_of(creds)?;
        let account = Self::account_of(creds);

        let resp = Self::client()?
            .get(format!("{BASE}/api/v10/library?page={page}"))
            .bearer_auth(&token)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(LibationError::ApiRequestFailed {
                message: format!("Libro.fm library failed ({})", resp.status()),
                status_code: Some(resp.status().as_u16()),
                endpoint: Some("/api/v10/library".into()),
            });
        }
        let lib: LibroLibrary = resp.json().await?;

        let mut added = 0i32;
        let mut updated = 0i32;
        for b in &lib.audiobooks {
            let existed = queries::find_book_by_asin(db.pool(), &b.isbn).await?.is_some();
            let (narrators, duration) = b
                .audiobook_info
                .as_ref()
                .map(|i| (i.narrators.clone(), i.duration))
                .unwrap_or((Vec::new(), 0));
            let cover = b.cover_url.as_deref().map(absolute_cover_url);
            queries::insert_libro_book(
                db.pool(),
                &b.isbn,
                &b.title,
                &b.authors,
                &narrators,
                b.description.as_deref().unwrap_or(""),
                duration / 60,
                cover.as_deref(),
                &account,
            )
            .await?;
            if existed {
                updated += 1;
            } else {
                added += 1;
            }
        }

        Ok(SyncStats {
            total_items: lib.audiobooks.len() as i32,
            total_library_count: 0, // not provided per page
            books_added: added,
            books_updated: updated,
            books_absent: 0,
            errors: Vec::new(),
            has_more: page < lib.total_pages,
        })
    }

    async fn download_plan(
        &self,
        _db: &Database,
        creds: &CredentialBlob,
        item_ref: &str,
        options: &PlanOptions,
    ) -> Result<DownloadPlan> {
        let token = Self::token_of(creds)?;
        // `{"format":"parts"}` gives a folder of MP3s (one zip per part);
        // anything else (the default) gives the single packaged M4B.
        let parts_folder = options
            .get("format")
            .and_then(|v| v.as_str())
            .is_some_and(|f| f == "parts");

        if parts_folder {
            let resp = Self::client()?
                .get(format!("{BASE}/api/v10/download-manifest?isbn={item_ref}"))
                .bearer_auth(&token)
                .send()
                .await?;
            if !resp.status().is_success() {
                return Err(LibationError::ApiRequestFailed {
                    message: format!("Libro.fm download-manifest failed ({})", resp.status()),
                    status_code: Some(resp.status().as_u16()),
                    endpoint: Some("/api/v10/download-manifest".into()),
                });
            }
            let manifest: DownloadManifest = resp.json().await?;
            if manifest.parts.is_empty() {
                return Err(LibationError::ApiRequestFailed {
                    message: "Libro.fm download-manifest returned no parts".into(),
                    status_code: None,
                    endpoint: Some("/api/v10/download-manifest".into()),
                });
            }
            // The signed asset URLs are pre-authorized — no headers needed.
            return Ok(DownloadPlan {
                parts: manifest
                    .parts
                    .iter()
                    .map(|p| DownloadPart::Zip {
                        url: p.url.clone(),
                        headers: Default::default(),
                    })
                    .collect(),
                embed_metadata: false,
                chapters: Self::chapters_from(&manifest.tracks),
            });
        }

        let resp = Self::client()?
            .get(format!("{BASE}/api/v10/audiobooks/{item_ref}/packaged_m4b"))
            .bearer_auth(&token)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(LibationError::ApiRequestFailed {
                message: format!("Libro.fm packaged_m4b failed ({})", resp.status()),
                status_code: Some(resp.status().as_u16()),
                endpoint: Some("packaged_m4b".into()),
            });
        }
        let m4b: M4bMeta = resp.json().await?;

        Ok(DownloadPlan {
            parts: vec![DownloadPart::Plain {
                url: m4b.m4b_url,
                headers: Default::default(),
                filename: format!("{item_ref}.m4b"),
            }],
            embed_metadata: false,
            chapters: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isbn_accepts_number_or_string() {
        #[derive(Deserialize)]
        struct T {
            #[serde(deserialize_with = "isbn_string")]
            isbn: String,
        }
        let n: T = serde_json::from_str(r#"{"isbn": 9798217174331}"#).unwrap();
        assert_eq!(n.isbn, "9798217174331");
        let s: T = serde_json::from_str(r#"{"isbn": "9798217174331"}"#).unwrap();
        assert_eq!(s.isbn, "9798217174331");
    }

    #[test]
    fn cover_url_gets_a_scheme() {
        assert_eq!(
            absolute_cover_url("//covers.libro.fm/123_1120.jpg"),
            "https://covers.libro.fm/123_1120.jpg"
        );
        assert_eq!(
            absolute_cover_url("https://covers.libro.fm/123.jpg"),
            "https://covers.libro.fm/123.jpg"
        );
    }

    #[test]
    fn chapters_accumulate_track_durations() {
        let tracks = vec![
            ManifestTrack {
                length_msec: 12_368,
                chapter_title: Some("Intro".into()),
                number: 1,
            },
            ManifestTrack {
                length_msec: 1_000,
                chapter_title: None,
                number: 2,
            },
        ];
        let ch = LibrofmProvider::chapters_from(&tracks);
        assert_eq!(ch[0].start_ms, 0);
        assert_eq!(ch[0].end_ms, 12_368);
        assert_eq!(ch[1].start_ms, 12_368);
        assert_eq!(ch[1].end_ms, 13_368);
        assert_eq!(ch[1].title, "Track 2");
    }
}
