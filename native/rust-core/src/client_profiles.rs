//! Client identity strings that stores gate on, updatable without an app release.
//!
//! Stores check how a client presents itself. Chirp started withholding every audio URL
//! from anything whose User-Agent isn't shaped like its own app (2026-11); fixing that
//! took a release, a Play review and users updating, while the fix was one string.
//! These strings now live in `client-profiles.json` at the root of the public repo,
//! which the app fetches at most daily and stores in the `ClientProfiles` table.
//!
//! **Scope is deliberately narrow:** header *values* only, from an allowlist, each
//! printable ASCII under a length cap. The file can never name a URL, a host, a
//! device type or anything sent at Audible registration — so the worst a bad file can
//! do is make requests fail, never send credentials somewhere. (Registration values are
//! bound into each account at sign-in; changing them remotely would make requests
//! disagree with the registration.) It is not signed: whoever can push to the repo can
//! already publish a release the in-app updater installs, so a signature would guard
//! nothing that the allowlist doesn't.
//!
//! Precedence: the file's values apply only when its `issued` date is on or after
//! [`COMPILED_ISSUED`], so an app update carrying newer strings is never overridden by
//! an older file. **Bump `COMPILED_ISSUED` whenever you change a compiled default**,
//! and keep `client-profiles.json` in step.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::storage::Database;
use crate::{LibationError, Result};

/// Where the profiles are published. Raw file on the default branch of the public repo.
pub const PROFILES_URL: &str =
    "https://raw.githubusercontent.com/Promises/LibriSync/main/client-profiles.json";

/// The file format this build understands. A file with a higher schema is ignored.
const SCHEMA: u32 = 1;

/// The date the compiled defaults below were last checked against the stores.
pub const COMPILED_ISSUED: &str = "2026-11-21";

/// How often the app checks for a new file in normal use.
pub const DAILY: chrono::Duration = chrono::Duration::hours(24);

/// How often a store's rejection symptom may trigger a check: often enough to pick up a
/// fix quickly, rarely enough that a batch download doesn't fetch once per book.
pub const ON_REJECTION: chrono::Duration = chrono::Duration::minutes(10);

const MAX_VALUE_LEN: usize = 200;
const MAX_FILE_BYTES: usize = 64 * 1024;

/// The effective identity strings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ClientProfiles {
    /// The `issued` date of whichever source these came from.
    pub issued: String,
    /// `true` when these came from the published file rather than the compiled defaults.
    pub remote: bool,
    /// Chirp GraphQL. Must have the app's `Chirp/<platform>/<version>` shape, or every
    /// `mediaUrl` comes back null.
    pub chirp_user_agent: String,
    /// Libro.fm API. Its edge 401s anything that doesn't look like the mobile app.
    pub librofm_user_agent: String,
    /// Libro.fm `X-LibroFm-AppVer`.
    pub librofm_app_version: String,
    /// Audible API requests (`api/client.rs`).
    pub audible_api_user_agent: String,
    /// Audible content downloads (the CDN fetch of the licensed file).
    pub audible_download_user_agent: String,
}

impl ClientProfiles {
    /// The defaults built into this app version.
    pub fn compiled() -> Self {
        Self {
            issued: COMPILED_ISSUED.to_string(),
            remote: false,
            chirp_user_agent: "Chirp/android/5.23.2".into(),
            librofm_user_agent: "okhttp/5.3.2".into(),
            librofm_app_version: "7.34.8".into(),
            audible_api_user_agent:
                "Dalvik/2.1.0 (Linux; U; Android 14) com.audible.application 26.34.07 b:2090263407"
                    .into(),
            audible_download_user_agent: "Audible/671 CFNetwork/1240.0.4 Darwin/20.6.0".into(),
        }
    }
}

// ── The published file ──────────────────────────────────────────────────────────────
//
// Unknown keys are ignored, so a newer file can add a store without breaking older apps.
// Every listed key is optional; a missing one keeps the compiled default.

#[derive(Deserialize)]
struct RemoteFile {
    schema: u32,
    issued: String,
    #[serde(default)]
    chirp: RemoteChirp,
    #[serde(default)]
    librofm: RemoteLibrofm,
    #[serde(default)]
    audible: RemoteAudible,
}

#[derive(Deserialize, Default)]
struct RemoteChirp {
    user_agent: Option<String>,
}

#[derive(Deserialize, Default)]
struct RemoteLibrofm {
    user_agent: Option<String>,
    app_version: Option<String>,
}

#[derive(Deserialize, Default)]
struct RemoteAudible {
    api_user_agent: Option<String>,
    download_user_agent: Option<String>,
}

/// A header value we are willing to send: printable ASCII, no control characters.
fn checked(field: &str, value: Option<String>, default: String) -> Result<String> {
    let Some(value) = value else {
        return Ok(default);
    };
    let ok = !value.is_empty()
        && value.len() <= MAX_VALUE_LEN
        && value.chars().all(|c| (' '..='~').contains(&c));
    if ok {
        Ok(value)
    } else {
        Err(LibationError::InvalidInput(format!(
            "client profiles: `{field}` is not a valid header value"
        )))
    }
}

/// Parse and validate a published file, filling gaps from the compiled defaults.
///
/// One bad value rejects the whole file: a half-applied file is harder to reason about
/// than a rejected one, and a bad value means the file is wrong anyway.
pub fn parse(body: &str) -> Result<ClientProfiles> {
    let file: RemoteFile = serde_json::from_str(body)
        .map_err(|e| LibationError::InvalidInput(format!("client profiles: {e}")))?;
    if file.schema != SCHEMA {
        return Err(LibationError::InvalidInput(format!(
            "client profiles: schema {} is not supported (this app reads {SCHEMA})",
            file.schema
        )));
    }
    NaiveDate::parse_from_str(&file.issued, "%Y-%m-%d").map_err(|_| {
        LibationError::InvalidInput(format!(
            "client profiles: issued `{}` is not YYYY-MM-DD",
            file.issued
        ))
    })?;

    let d = ClientProfiles::compiled();
    Ok(ClientProfiles {
        issued: file.issued,
        remote: true,
        chirp_user_agent: checked("chirp.user_agent", file.chirp.user_agent, d.chirp_user_agent)?,
        librofm_user_agent: checked(
            "librofm.user_agent",
            file.librofm.user_agent,
            d.librofm_user_agent,
        )?,
        librofm_app_version: checked(
            "librofm.app_version",
            file.librofm.app_version,
            d.librofm_app_version,
        )?,
        audible_api_user_agent: checked(
            "audible.api_user_agent",
            file.audible.api_user_agent,
            d.audible_api_user_agent,
        )?,
        audible_download_user_agent: checked(
            "audible.download_user_agent",
            file.audible.download_user_agent,
            d.audible_download_user_agent,
        )?,
    })
}

/// The published profiles if they are at least as new as the compiled ones, else the
/// compiled ones. ISO dates compare correctly as strings.
fn effective(published: Option<ClientProfiles>) -> ClientProfiles {
    match published {
        Some(p) if p.issued.as_str() >= COMPILED_ISSUED => p,
        _ => ClientProfiles::compiled(),
    }
}

// ── Process-wide current value ──────────────────────────────────────────────────────

fn cell() -> &'static RwLock<ClientProfiles> {
    static CURRENT: OnceLock<RwLock<ClientProfiles>> = OnceLock::new();
    CURRENT.get_or_init(|| RwLock::new(ClientProfiles::compiled()))
}

static LOADED: AtomicBool = AtomicBool::new(false);

/// The profiles every client should use right now.
pub fn current() -> ClientProfiles {
    cell().read().map(|p| p.clone()).unwrap_or_else(|_| ClientProfiles::compiled())
}

fn set_current(profiles: ClientProfiles) {
    if let Ok(mut guard) = cell().write() {
        *guard = profiles;
    }
}

// ── Storage ─────────────────────────────────────────────────────────────────────────

/// The stored file body and when it was fetched.
async fn read_stored(db: &Database) -> Result<Option<(String, DateTime<Utc>)>> {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT body, fetched_at FROM ClientProfiles WHERE id = 1")
            .fetch_optional(db.pool())
            .await?;
    Ok(row.and_then(|(body, at)| {
        DateTime::parse_from_rfc3339(&at)
            .ok()
            .map(|at| (body, at.with_timezone(&Utc)))
    }))
}

async fn store(db: &Database, body: &str, fetched_at: DateTime<Utc>) -> Result<()> {
    sqlx::query(
        "INSERT INTO ClientProfiles (id, body, fetched_at) VALUES (1, ?, ?)
         ON CONFLICT(id) DO UPDATE SET body = excluded.body, fetched_at = excluded.fetched_at",
    )
    .bind(body)
    .bind(fetched_at.to_rfc3339())
    .execute(db.pool())
    .await?;
    Ok(())
}

/// Adopt the stored profiles, once per process. Called when a database is opened, so
/// background workers that never run JS still use them. Failures leave the compiled
/// defaults in place: a missing or unreadable copy must never break a request.
pub async fn load_once(db: &Database) {
    if LOADED.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Ok(Some((body, _))) = read_stored(db).await {
        if let Ok(published) = parse(&body) {
            set_current(effective(Some(published)));
        }
    }
}

/// What [`refresh`] did, for the bridge and logs.
#[derive(Debug, Serialize)]
pub struct RefreshOutcome {
    /// Whether the file was downloaded this time (false = the stored copy was fresh).
    pub fetched: bool,
    /// Whether the effective profiles changed.
    pub changed: bool,
    pub profiles: ClientProfiles,
}

/// Fetch the published file if the stored copy is older than `max_age` ([`DAILY`] or
/// [`ON_REJECTION`]), store it, and adopt it. A failed fetch keeps whatever is in use
/// and returns the error.
pub async fn refresh(db: &Database, max_age: chrono::Duration) -> Result<RefreshOutcome> {
    load_once(db).await;
    let before = current();

    if let Some((_, fetched_at)) = read_stored(db).await? {
        if Utc::now() - fetched_at < max_age {
            return Ok(RefreshOutcome { fetched: false, changed: false, profiles: before });
        }
    }

    let body = fetch().await?;
    let published = parse(&body)?;
    store(db, &body, Utc::now()).await?;
    let after = effective(Some(published));
    set_current(after.clone());
    Ok(RefreshOutcome { fetched: true, changed: after != before, profiles: after })
}

async fn fetch() -> Result<String> {
    let failed = |message: String| LibationError::ApiRequestFailed {
        message,
        status_code: None,
        endpoint: Some(PROFILES_URL.into()),
    };
    let client = reqwest::Client::builder()
        .user_agent("LibriSync")
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| failed(format!("client profiles: {e}")))?;
    let response = client
        .get(PROFILES_URL)
        .send()
        .await
        .map_err(|e| failed(format!("client profiles: {e}")))?;
    if !response.status().is_success() {
        return Err(failed(format!("client profiles: HTTP {}", response.status())));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| failed(format!("client profiles: {e}")))?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(failed("client profiles: file too large".into()));
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| failed("client profiles: not UTF-8".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_published_file_parses_and_matches_the_compiled_defaults() {
        // The repo's copy must always be valid, and must not lag the compiled values.
        let body = include_str!("../../../client-profiles.json");
        let mut published = parse(body).expect("client-profiles.json is valid");
        assert!(published.issued.as_str() >= COMPILED_ISSUED, "file is older than the build");
        published.issued = COMPILED_ISSUED.into();
        published.remote = false;
        assert_eq!(published, ClientProfiles::compiled(), "file and compiled defaults differ");
    }

    #[test]
    fn missing_keys_keep_the_defaults_and_unknown_keys_are_ignored() {
        let p = parse(
            r#"{"schema":1,"issued":"2099-01-01",
                "chirp":{"user_agent":"Chirp/android/9.9.9"},
                "newstore":{"user_agent":"x"}}"#,
        )
        .unwrap();
        assert_eq!(p.chirp_user_agent, "Chirp/android/9.9.9");
        assert_eq!(p.librofm_user_agent, ClientProfiles::compiled().librofm_user_agent);
        assert!(p.remote);
    }

    #[test]
    fn a_bad_value_rejects_the_whole_file() {
        for bad in ["", "a\r\nX-Injected: 1", "caf\u{e9}", &"x".repeat(MAX_VALUE_LEN + 1)] {
            let body = serde_json::json!({
                "schema": 1, "issued": "2099-01-01",
                "chirp": {"user_agent": "Chirp/android/9.9.9"},
                "librofm": {"app_version": bad},
            })
            .to_string();
            assert!(parse(&body).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn an_unknown_schema_or_bad_date_is_refused() {
        assert!(parse(r#"{"schema":2,"issued":"2099-01-01"}"#).is_err());
        assert!(parse(r#"{"schema":1,"issued":"yesterday"}"#).is_err());
        assert!(parse(r#"{"issued":"2099-01-01"}"#).is_err());
    }

    #[test]
    fn an_older_file_never_overrides_newer_compiled_defaults() {
        let old = parse(r#"{"schema":1,"issued":"2020-01-01","chirp":{"user_agent":"old"}}"#)
            .unwrap();
        assert_eq!(effective(Some(old)), ClientProfiles::compiled());

        let new = parse(r#"{"schema":1,"issued":"2099-01-01","chirp":{"user_agent":"new"}}"#)
            .unwrap();
        assert_eq!(effective(Some(new)).chirp_user_agent, "new");
        assert_eq!(effective(None), ClientProfiles::compiled());
    }

    #[tokio::test]
    async fn the_stored_copy_round_trips() {
        let db = Database::new_in_memory().await.unwrap();
        assert!(read_stored(&db).await.unwrap().is_none());
        let at = Utc::now();
        store(&db, "{\"a\":1}", at).await.unwrap();
        store(&db, "{\"a\":2}", at).await.unwrap();
        let (body, fetched_at) = read_stored(&db).await.unwrap().unwrap();
        assert_eq!(body, "{\"a\":2}");
        assert_eq!(fetched_at.timestamp(), at.timestamp());
    }
}
