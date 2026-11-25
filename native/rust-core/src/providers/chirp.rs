//! Chirp provider (chirpbooks.com) — a DRM-free audiobook store.
//!
//! Chirp's Android app talks to one GraphQL endpoint. Sign-in is a `signIn`
//! mutation that returns a JWT (≈1 year lifetime, no refresh token); the library is
//! the paginated `currentUserAudiobooks` query; a book's audio is a list of `tracks`,
//! each a plain AAC `.m4a` behind a signed `mediaUrl` that needs no headers.
//!
//! The API surface was found by Bookclerk's reverse-engineering of the Android APK
//! (github.com/Bookclerk/bookclerk, `source-chirp`). No code is taken from it — it is
//! AGPL — only the protocol facts, each of which was re-verified against the live API
//! on 2026-11-04:
//!
//! * the server validates the query against its schema *before* auth, so a field typo
//!   errors even without a token;
//! * auth failures come back as HTTP 200 with `{"errors":[{"code":"unauthorized"}]}` or
//!   `"token_invalid"` — a `code`, no `message`;
//! * pages are 1-based, and page 0 silently returns page 1;
//! * `mediaUrl`s expire ~24 h after they are issued. (ch.rip's 60 s expiry applies to
//!   the *web player's* URLs, not these.)
//!
//! Chirp has no "download the book" endpoint: a book is its tracks, so the download
//! plan is one `Plain` part per track with `join_parts` set, and the engine joins them
//! into one file with a chapter per track.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};

use crate::api::library::SyncStats;
use crate::client_profiles;
use crate::storage::{queries, Database};
use crate::{LibationError, Result};

use super::http;
use super::{
    CredentialBlob, DownloadPart, DownloadPlan, LoginInput, PlanChapter, PlanOptions, Provider,
    ProviderId,
};

const GRAPHQL_URL: &str = "https://api.chirpbooks.com/api/graphql";


/// Prefix for Chirp ids in `Books.audible_product_id`. Chirp ids are short integers
/// ("224712") and that column also holds ASINs, ISBNs and LibriVox ids, so an
/// unprefixed id could collide with another source's book.
const PRODUCT_PREFIX: &str = "chirp_";

/// Prefix for `Accounts.account_id`. Libro.fm keys accounts by bare email, so a
/// person with the same email at both stores would otherwise have one account
/// overwrite the other.
const ACCOUNT_PREFIX: &str = "chirp:";

/// The server shows no page-size cap, but the end of the library is detected by an
/// empty page (see `sync_library_page`), so a cap we can't see costs a request, not
/// books.
const PAGE_SIZE: i32 = 100;

const SIGN_IN: &str = r#"
mutation signIn($email: String!, $password: String!) {
  signIn(email: $email, password: $password) { user { id token email } }
}"#;

const LIBRARY_PAGE: &str = r#"
query AndroidCurrentUserAudiobooks($page: Int!, $pageSize: Int!) {
  currentUserAudiobooks(page: $page, pageSize: $pageSize, sort: TITLE_A_Z,
                        clientCapabilities: [CHIRP_AUDIO]) {
    id
    playable
    audiobook {
      id displayTitle subTitle displayAuthors displayNarrators durationMs abridged
      publisher releasedOn language description
      coverUrl: optimizedCoverUrl(format: "f_jpg", quality: "q_auto:eco", sizePixels: 700)
      seriesAudiobook { displayNumber series { id name } }
    }
  }
}"#;

const SINGLE_AUDIOBOOK: &str = r#"
query AndroidSingleAudiobook($id: ID!) {
  audiobook(id: $id, clientCapabilities: [CHIRP_AUDIO]) {
    id
    tracks { id mediaUrl durationMs offsetFromBookStartMs displayName }
  }
}"#;

pub struct ChirpProvider;

/// GraphQL `ID`s arrive as strings today; accept a number too, defensively.
fn id_string<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<String, D::Error> {
    match Value::deserialize(d)? {
        Value::String(s) => Ok(s),
        Value::Number(n) => Ok(n.to_string()),
        other => Err(serde::de::Error::custom(format!("unexpected id value: {other}"))),
    }
}

#[derive(Deserialize)]
struct SignInUser {
    #[serde(deserialize_with = "id_string")]
    id: String,
    token: String,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Deserialize)]
struct UserAudiobook {
    #[serde(default)]
    playable: Option<bool>,
    #[serde(default)]
    audiobook: Option<Audiobook>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Audiobook {
    #[serde(deserialize_with = "id_string")]
    id: String,
    #[serde(default)]
    display_title: Option<String>,
    #[serde(default)]
    sub_title: Option<String>,
    #[serde(default)]
    display_authors: Option<String>,
    #[serde(default)]
    display_narrators: Option<String>,
    #[serde(default)]
    duration_ms: Option<u64>,
    #[serde(default)]
    abridged: Option<bool>,
    #[serde(default)]
    publisher: Option<String>,
    #[serde(default)]
    released_on: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    cover_url: Option<String>,
    #[serde(default)]
    series_audiobook: Option<SeriesAudiobook>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeriesAudiobook {
    #[serde(default)]
    display_number: Option<String>,
    series: SeriesRef,
}

#[derive(Deserialize)]
struct SeriesRef {
    #[serde(deserialize_with = "id_string")]
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct BookTracks {
    #[serde(default)]
    tracks: Vec<Track>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Track {
    #[serde(deserialize_with = "id_string")]
    id: String,
    #[serde(default)]
    media_url: Option<String>,
    #[serde(default)]
    duration_ms: Option<u64>,
    #[serde(default)]
    offset_from_book_start_ms: Option<u64>,
    #[serde(default)]
    display_name: Option<String>,
}

impl ChirpProvider {
    /// The User-Agent is load-bearing: since 2026-11 Chirp returns every `mediaUrl` as
    /// null unless it has the app's `Chirp/<platform>/<version>` shape. It comes from
    /// [`client_profiles`] so it can be changed without a release.
    fn client() -> Result<Client> {
        http::client(&client_profiles::current().chirp_user_agent)
    }

    /// POST one GraphQL operation and return its `data` object.
    ///
    /// Chirp reports every failure — bad password, bad token, unknown book — as HTTP
    /// 200 with an `errors` array, so the array is checked before `data` is trusted.
    async fn graphql(
        operation: &str,
        query: &str,
        variables: Value,
        token: Option<&str>,
    ) -> Result<Value> {
        let mut request = Self::client()?.post(GRAPHQL_URL).json(&json!({
            "operationName": operation,
            "query": query,
            "variables": variables,
        }));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let resp = request.send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(LibationError::ApiRequestFailed {
                message: format!("Chirp {operation} failed ({status})"),
                status_code: Some(status.as_u16()),
                endpoint: Some(operation.to_string()),
            });
        }
        let body: Value = resp.json().await?;
        Self::data_of(operation, body)
    }

    /// Split a GraphQL response into its `data`, or the error it carries. Partial data
    /// alongside errors is accepted — only a response with nothing usable fails.
    fn data_of(operation: &str, body: Value) -> Result<Value> {
        let data = body.get("data").cloned().unwrap_or(Value::Null);
        let errors = body
            .get("errors")
            .and_then(Value::as_array)
            .filter(|e| !e.is_empty());
        let Some(errors) = errors else {
            return Ok(data);
        };
        let has_data = data
            .as_object()
            .is_some_and(|o| o.values().any(|v| !v.is_null()));
        if has_data {
            return Ok(data);
        }

        let codes: Vec<&str> = errors
            .iter()
            .filter_map(|e| e.get("code").and_then(Value::as_str))
            .collect();
        if codes.iter().any(|c| matches!(*c, "unauthorized" | "token_invalid")) {
            return Err(LibationError::AuthenticationFailed {
                message: "Chirp session expired. Sign in to Chirp again from the Accounts tab."
                    .into(),
                account_id: None,
            });
        }
        let messages: Vec<&str> = errors
            .iter()
            .filter_map(|e| e.get("message").and_then(Value::as_str))
            .collect();
        let detail = if messages.is_empty() {
            codes.join(", ")
        } else {
            messages.join("; ")
        };
        Err(LibationError::ApiRequestFailed {
            message: format!("Chirp {operation}: {detail}"),
            status_code: None,
            endpoint: Some(operation.to_string()),
        })
    }

    /// The bearer token, whether `creds` is the raw credential blob or a full account
    /// (`{identity: {access_token, ..}}`) — the bridge passes either.
    fn token_of(creds: &CredentialBlob) -> Result<String> {
        creds
            .get("access_token")
            .or_else(|| creds.get("identity").and_then(|i| i.get("access_token")))
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or_else(|| LibationError::AuthenticationFailed {
                message: "Missing Chirp access token".into(),
                account_id: None,
            })
    }

    /// The `Accounts.account_id` that owns this library — books are linked to it.
    fn account_of(creds: &CredentialBlob) -> Result<String> {
        creds
            .get("account_id")
            .or_else(|| creds.get("identity").and_then(|i| i.get("account_id")))
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or_else(|| LibationError::InvalidInput("Chirp account has no account_id".into()))
    }

    fn product_id(chirp_id: &str) -> String {
        format!("{PRODUCT_PREFIX}{chirp_id}")
    }

    /// Chirp gives contributors as one display string ("A, B and C"). Splits it into
    /// names, re-attaching suffixes like "Jr." that a comma split would orphan.
    fn split_names(display: Option<&str>) -> Vec<String> {
        const SUFFIXES: [&str; 6] = ["jr", "jr.", "sr", "sr.", "phd", "md"];
        let mut names: Vec<String> = Vec::new();
        let Some(display) = display else {
            return names;
        };
        for piece in display.split([',', ';', '&']).flat_map(|p| p.split(" and ")) {
            let piece = piece.trim();
            if piece.is_empty() {
                continue;
            }
            if SUFFIXES.contains(&piece.to_ascii_lowercase().as_str()) {
                if let Some(last) = names.last_mut() {
                    last.push_str(", ");
                    last.push_str(piece);
                    continue;
                }
            }
            names.push(piece.to_string());
        }
        names
    }

    /// A book's tracks, as the app's `AndroidSingleAudiobook` query returns them.
    async fn fetch_tracks(token: &str, chirp_id: &str) -> Result<Vec<Track>> {
        let data = Self::graphql(
            "AndroidSingleAudiobook",
            SINGLE_AUDIOBOOK,
            json!({ "id": chirp_id }),
            Some(token),
        )
        .await?;
        match data.get("audiobook") {
            Some(v) if !v.is_null() => Ok(serde_json::from_value::<BookTracks>(v.clone())?.tracks),
            _ => Err(LibationError::ApiRequestFailed {
                message: format!("Chirp book {chirp_id} not found"),
                status_code: None,
                endpoint: Some("AndroidSingleAudiobook".into()),
            }),
        }
    }

    /// The tracks in playback order, as plan parts plus one chapter each.
    ///
    /// Ordered by `offsetFromBookStartMs` rather than trusting response order. A track
    /// without a `mediaUrl` fails the plan: a joined book with a hole in it is worse
    /// than an error.
    fn plan_from_tracks(chirp_id: &str, mut tracks: Vec<Track>) -> Result<DownloadPlan> {
        if tracks.is_empty() {
            return Err(LibationError::ApiRequestFailed {
                message: format!("Chirp returned no audio tracks for book {chirp_id}"),
                status_code: None,
                endpoint: Some("AndroidSingleAudiobook".into()),
            });
        }
        // No URL on *any* track is the server refusing this client (see `client()`),
        // not a damaged book — say so, rather than blaming one track.
        if tracks.iter().all(|t| t.media_url.is_none()) {
            return Err(LibationError::ApiRequestFailed {
                message: format!(
                    "Chirp did not provide download links for book {chirp_id}. \
                     Chirp may have changed how its app fetches audio; \
                     a LibriSync update is likely needed."
                ),
                status_code: None,
                endpoint: Some("AndroidSingleAudiobook".into()),
            });
        }
        tracks.sort_by_key(|t| t.offset_from_book_start_ms.unwrap_or(0));

        let mut parts = Vec::with_capacity(tracks.len());
        let mut chapters = Vec::with_capacity(tracks.len());
        let mut cursor = 0u64;
        for (i, track) in tracks.iter().enumerate() {
            let url = track.media_url.clone().ok_or_else(|| LibationError::ApiRequestFailed {
                message: format!(
                    "Chirp track {} of book {chirp_id} has no audio URL",
                    track.id
                ),
                status_code: None,
                endpoint: Some("AndroidSingleAudiobook".into()),
            })?;
            let extension = Self::audio_extension(&url);
            parts.push(DownloadPart::Plain {
                url,
                headers: Default::default(),
                filename: format!("{:03}.{extension}", i + 1),
            });

            let start = track.offset_from_book_start_ms.unwrap_or(cursor);
            let end = start + track.duration_ms.unwrap_or(0);
            cursor = end;
            chapters.push(PlanChapter {
                title: track
                    .display_name
                    .clone()
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| format!("Track {}", i + 1)),
                start_ms: start,
                end_ms: end,
            });
        }

        Ok(DownloadPlan {
            parts,
            embed_metadata: true,
            chapters,
            join_parts: true,
            licence_throttled: false,
        })
    }

    /// Chirp serves AAC `.m4a` for most books and the occasional `.mp3`. The engine
    /// picks the joined container from this, so it must not default to the wrong one.
    fn audio_extension(url: &str) -> &'static str {
        let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
        if path.ends_with(".mp3") {
            "mp3"
        } else {
            "m4a"
        }
    }
}

#[async_trait]
impl Provider for ChirpProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Chirp
    }

    async fn login(&self, input: LoginInput) -> Result<CredentialBlob> {
        let email = input
            .get("username")
            .and_then(Value::as_str)
            .ok_or_else(|| LibationError::InvalidInput("email required".into()))?;
        let password = input
            .get("password")
            .and_then(Value::as_str)
            .ok_or_else(|| LibationError::InvalidInput("password required".into()))?;

        let data = Self::graphql(
            "signIn",
            SIGN_IN,
            json!({ "email": email, "password": password }),
            None,
        )
        .await
        .map_err(|e| match e {
            // A wrong password is an ordinary GraphQL error; surface it as an auth failure.
            LibationError::ApiRequestFailed { message, .. } => LibationError::AuthenticationFailed {
                message: message.replace("Chirp signIn: ", "Chirp sign-in failed: "),
                account_id: Some(email.to_string()),
            },
            other => other,
        })?;

        let user: SignInUser = data
            .pointer("/signIn/user")
            .cloned()
            .map(serde_json::from_value)
            .transpose()?
            .filter(|u: &SignInUser| !u.token.is_empty())
            .ok_or_else(|| LibationError::AuthenticationFailed {
                message: "Chirp returned no session token".into(),
                account_id: Some(email.to_string()),
            })?;

        // `webToken` is deliberately not kept: nothing here uses it, and an unused
        // credential is still one more secret in the database.
        Ok(json!({
            "access_token": user.token,
            "account_id": format!("{ACCOUNT_PREFIX}{}", user.id),
            "username": user.email.unwrap_or_else(|| email.to_string()),
            "user_id": user.id,
        }))
    }

    async fn refresh(&self, _creds: &CredentialBlob) -> Result<Option<CredentialBlob>> {
        // No refresh grant exists; the JWT lasts about a year. Expiry surfaces as an
        // AuthenticationFailed asking the user to sign in again.
        Ok(None)
    }

    async fn sync_library_page(
        &self,
        db: &Database,
        creds: &CredentialBlob,
        page: i32,
    ) -> Result<SyncStats> {
        let token = Self::token_of(creds)?;
        let account = Self::account_of(creds)?;

        let data = Self::graphql(
            "AndroidCurrentUserAudiobooks",
            LIBRARY_PAGE,
            json!({ "page": page.max(1), "pageSize": PAGE_SIZE }),
            Some(&token),
        )
        .await?;
        let items: Vec<Value> = data
            .get("currentUserAudiobooks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        let mut stats = SyncStats {
            total_items: items.len() as i32,
            total_library_count: 0, // Chirp gives no total
            books_added: 0,
            books_updated: 0,
            books_absent: 0,
            errors: Vec::new(),
            items_failed: 0,
            // No total either, and a short page might just be a server-side cap, so
            // only an empty page ends the library.
            has_more: !items.is_empty(),
        };

        // One item at a time, so one odd item costs that item and not the page.
        for item in items {
            let parsed: UserAudiobook = match serde_json::from_value(item) {
                Ok(p) => p,
                Err(e) => {
                    stats.items_failed += 1;
                    stats.errors.push(format!("Chirp item unreadable: {e}"));
                    continue;
                }
            };
            let Some(book) = parsed.audiobook else {
                stats.items_failed += 1;
                stats.errors.push("Chirp library entry without an audiobook".into());
                continue;
            };

            let product_id = Self::product_id(&book.id);
            let title = book.display_title.clone().unwrap_or_else(|| product_id.clone());
            let authors = Self::split_names(book.display_authors.as_deref());
            let narrators = Self::split_names(book.display_narrators.as_deref());
            let language = book
                .language
                .as_deref()
                .unwrap_or("english")
                .to_ascii_lowercase();
            let series_key = book
                .series_audiobook
                .as_ref()
                .map(|s| format!("{PRODUCT_PREFIX}series_{}", s.series.id));
            let series = book.series_audiobook.as_ref().zip(series_key.as_deref()).map(
                |(s, key)| queries::StoreSeries {
                    series_key: key,
                    name: &s.series.name,
                    order: s.display_number.as_deref(),
                },
            );

            let (_, inserted) = queries::insert_store_book(
                db.pool(),
                &queries::StoreBook {
                    source: ProviderId::Chirp.as_str(),
                    product_id: &product_id,
                    title: &title,
                    subtitle: book.sub_title.as_deref(),
                    authors: &authors,
                    narrators: &narrators,
                    publisher: book.publisher.as_deref(),
                    description: book.description.as_deref().unwrap_or(""),
                    length_in_minutes: (book.duration_ms.unwrap_or(0) / 60_000) as i32,
                    cover_url: book.cover_url.as_deref(),
                    language: &language,
                    date_published: book.released_on.as_deref(),
                    is_abridged: book.abridged.unwrap_or(false),
                    is_downloadable: parsed.playable.unwrap_or(true),
                    series,
                    account: &account,
                },
            )
            .await?;
            if inserted {
                stats.books_added += 1;
            } else {
                stats.books_updated += 1;
            }
        }

        Ok(stats)
    }

    async fn download_plan(
        &self,
        db: &Database,
        creds: &CredentialBlob,
        item_ref: &str,
        _options: &PlanOptions,
    ) -> Result<DownloadPlan> {
        // The output format (single file vs MP3 per chapter) is applied by the engine
        // after joining, as for Audible, so Chirp has no per-download options.
        let token = Self::token_of(creds)?;
        let chirp_id = item_ref.strip_prefix(PRODUCT_PREFIX).unwrap_or(item_ref);

        let mut tracks = Self::fetch_tracks(&token, chirp_id).await?;
        if !tracks.is_empty() && tracks.iter().all(|t| t.media_url.is_none()) {
            // Chirp withholding every URL is how it rejects our User-Agent. A newer
            // published profile may already carry the fix: fetch it and retry once.
            let before = client_profiles::current().chirp_user_agent;
            // A failed refresh falls through to the plan error below, which already says
            // what went wrong.
            if let Ok(outcome) = client_profiles::refresh(db, client_profiles::ON_REJECTION).await {
                if outcome.profiles.chirp_user_agent != before {
                    tracks = Self::fetch_tracks(&token, chirp_id).await?;
                }
            }
        }
        Self::plan_from_tracks(chirp_id, tracks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str, offset: u64, duration: u64, name: Option<&str>) -> Track {
        Track {
            id: id.into(),
            media_url: Some(format!(
                "https://audio.chirpbooks.com/x/{id}.m4a?Expires=1&Signature=s&Key-Pair-Id=k"
            )),
            duration_ms: Some(duration),
            offset_from_book_start_ms: Some(offset),
            display_name: name.map(String::from),
        }
    }

    #[test]
    fn plan_orders_tracks_and_marks_one_chapter_each() {
        // Deliberately out of order: the plan must follow the offsets.
        let plan = ChirpProvider::plan_from_tracks(
            "224712",
            vec![
                track("b", 1_000, 500, Some("Track 2")),
                track("a", 0, 1_000, Some("Track 1")),
                track("c", 1_500, 250, None),
            ],
        )
        .unwrap();

        assert!(plan.join_parts);
        assert_eq!(plan.parts.len(), 3);
        let names: Vec<_> = plan
            .parts
            .iter()
            .map(|p| match p {
                DownloadPart::Plain { url, filename, .. } => (url.contains("/a."), filename.clone()),
                _ => panic!("Chirp plans are plain parts only"),
            })
            .collect();
        assert_eq!(names[0], (true, "001.m4a".to_string()));
        assert_eq!(names[2].1, "003.m4a");

        assert_eq!(plan.chapters[0].start_ms, 0);
        assert_eq!(plan.chapters[0].end_ms, 1_000);
        assert_eq!(plan.chapters[1].start_ms, 1_000);
        assert_eq!(plan.chapters[2].title, "Track 3");
        assert_eq!(plan.chapters[2].end_ms, 1_750);
    }

    #[test]
    fn plan_refuses_a_track_without_audio() {
        let mut missing = track("b", 1_000, 500, None);
        missing.media_url = None;
        let err = ChirpProvider::plan_from_tracks("1", vec![track("a", 0, 1_000, None), missing]);
        assert!(err.unwrap_err().to_string().contains("track b of book 1 has no audio URL"));
        assert!(ChirpProvider::plan_from_tracks("1", Vec::new()).is_err());
    }

    #[test]
    fn plan_blames_the_client_when_no_track_has_audio() {
        let tracks: Vec<Track> = ["a", "b"]
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let mut t = track(id, i as u64 * 1_000, 1_000, None);
                t.media_url = None;
                t
            })
            .collect();
        let err = ChirpProvider::plan_from_tracks("1", tracks).unwrap_err().to_string();
        assert!(err.contains("did not provide download links"), "{err}");
    }

    #[test]
    fn audio_extension_ignores_the_signed_query() {
        assert_eq!(ChirpProvider::audio_extension("https://a/b/c.mp3?Signature=x.m4a"), "mp3");
        assert_eq!(ChirpProvider::audio_extension("https://a/b/c.m4a?Expires=1"), "m4a");
    }

    #[test]
    fn auth_errors_carry_a_code_and_no_message() {
        // Verbatim shapes from the live API.
        for code in ["unauthorized", "token_invalid"] {
            let body = json!({ "errors": [{ "code": code }] });
            assert!(matches!(
                ChirpProvider::data_of("AndroidCurrentUserAudiobooks", body),
                Err(LibationError::AuthenticationFailed { .. })
            ));
        }
        let bad_password = json!({
            "errors": [{ "message": "Invalid username or password.", "path": ["signIn"] }],
            "data": { "signIn": null }
        });
        match ChirpProvider::data_of("signIn", bad_password) {
            Err(LibationError::ApiRequestFailed { message, .. }) => {
                assert!(message.contains("Invalid username or password."))
            }
            _ => panic!("expected the server's message"),
        }
        let ok = json!({ "data": { "currentUserAudiobooks": [] } });
        assert!(ChirpProvider::data_of("x", ok).is_ok());
    }

    #[test]
    fn library_item_parses_the_live_shape() {
        // Trimmed from a real response (2026-11-04).
        let item: UserAudiobook = serde_json::from_value(json!({
            "id": "38399676", "archived": false, "playable": true,
            "audiobook": {
                "id": "224712", "displayTitle": "Animal Farm", "subTitle": null,
                "displayAuthors": "George Orwell", "displayNarrators": "Ralph Cosham",
                "durationMs": 11507272, "abridged": false,
                "publisher": "Blackstone Publishing", "releasedOn": "2007-01-01",
                "language": "English", "description": "<p>…</p>",
                "coverUrl": "https://img.chirpbooks.com/image/upload/x.jpg",
                "seriesAudiobook": null
            }
        }))
        .unwrap();
        let book = item.audiobook.unwrap();
        assert_eq!(ChirpProvider::product_id(&book.id), "chirp_224712");
        assert_eq!(book.duration_ms, Some(11_507_272));
        assert!(book.series_audiobook.is_none());
    }

    #[test]
    fn names_split_on_separators_but_keep_suffixes() {
        assert_eq!(
            ChirpProvider::split_names(Some("Neil Gaiman, Terry Pratchett and Someone Else")),
            vec!["Neil Gaiman", "Terry Pratchett", "Someone Else"]
        );
        assert_eq!(
            ChirpProvider::split_names(Some("Martin Luther King, Jr.")),
            vec!["Martin Luther King, Jr."]
        );
        assert!(ChirpProvider::split_names(None).is_empty());
    }

    #[test]
    fn credentials_resolve_from_blob_or_account() {
        let blob = json!({ "access_token": "t", "account_id": "chirp:1" });
        let account = json!({ "account_id": "chirp:1", "identity": blob.clone() });
        assert_eq!(ChirpProvider::token_of(&blob).unwrap(), "t");
        assert_eq!(ChirpProvider::token_of(&account).unwrap(), "t");
        assert_eq!(ChirpProvider::account_of(&account).unwrap(), "chirp:1");
    }
}
