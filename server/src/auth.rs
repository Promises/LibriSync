//! Authentication: the single admin user (argon2), cookie sessions, and the API key.
//!
//! Every `/api/*` route except the anonymous ones below needs either a session cookie
//! or the API key (`X-Api-Key` header or `?apikey=` query). `GET /ping` and the
//! embedded web UI are anonymous.

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Uri};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use chrono::Duration;
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::error::ApiError;
use crate::state::AppState;
use crate::time;

pub const COOKIE: &str = "librisync_session";
pub const SESSION_DAYS: i64 = 30;
pub const MIN_PASSWORD: usize = 8;

const ANONYMOUS: &[&str] = &["/api/v1/auth/state", "/api/v1/auth/setup", "/api/v1/auth/login"];

/// Who is calling, attached to authenticated requests.
#[derive(Debug, Clone)]
pub struct Principal {
    /// The session's user; `None` when authenticated by API key.
    pub user: Option<(i64, String)>,
}

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| anyhow::anyhow!("hashing password: {e}"))
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|parsed| Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
        .unwrap_or(false)
}

/// Compare secrets without leaking where they differ.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub async fn user_count(pool: &SqlitePool) -> sqlx::Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM users").fetch_one(pool).await
}

pub async fn first_user(pool: &SqlitePool) -> sqlx::Result<Option<(i64, String, String)>> {
    sqlx::query_as("SELECT id, username, password_hash FROM users ORDER BY id LIMIT 1")
        .fetch_optional(pool)
        .await
}

pub async fn find_user(pool: &SqlitePool, username: &str) -> sqlx::Result<Option<(i64, String, String)>> {
    sqlx::query_as("SELECT id, username, password_hash FROM users WHERE username = ?")
        .bind(username)
        .fetch_optional(pool)
        .await
}

pub fn check_new_password(password: &str) -> Result<(), ApiError> {
    if password.chars().count() < MIN_PASSWORD {
        return Err(ApiError::invalid_field("password", format!("must be at least {MIN_PASSWORD} characters")));
    }
    Ok(())
}

/// Create the admin user, only while none exists. Returns the new user id, or `None`
/// if a user was created concurrently.
pub async fn create_first_user(pool: &SqlitePool, username: &str, password: &str) -> anyhow::Result<Option<i64>> {
    let hash = hash_password(password)?;
    let id = sqlx::query(
        "INSERT INTO users (username, password_hash, created_at)
         SELECT ?, ?, ? WHERE NOT EXISTS (SELECT 1 FROM users)",
    )
    .bind(username)
    .bind(hash)
    .bind(time::now_str())
    .execute(pool)
    .await?;
    Ok((id.rows_affected() == 1).then(|| id.last_insert_rowid()))
}

pub async fn set_password(pool: &SqlitePool, user_id: i64, password: &str) -> anyhow::Result<()> {
    let hash = hash_password(password)?;
    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(hash)
        .bind(user_id)
        .execute(pool)
        .await?;
    // Changing the password signs out every other session.
    sqlx::query("DELETE FROM sessions WHERE user_id = ?")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Start a session; returns the cookie token (only its hash is stored).
pub async fn create_session(pool: &SqlitePool, user_id: i64) -> sqlx::Result<String> {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token = hex::encode(bytes);
    let now = time::now();
    sqlx::query("INSERT INTO sessions (token_hash, user_id, created_at, expires_at) VALUES (?, ?, ?, ?)")
        .bind(token_hash(&token))
        .bind(user_id)
        .bind(time::fmt(now))
        .bind(time::fmt(now + Duration::days(SESSION_DAYS)))
        .execute(pool)
        .await?;
    Ok(token)
}

pub async fn session_user(pool: &SqlitePool, token: &str) -> sqlx::Result<Option<(i64, String)>> {
    sqlx::query_as(
        "SELECT u.id, u.username FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE s.token_hash = ? AND s.expires_at > ?",
    )
    .bind(token_hash(token))
    .bind(time::now_str())
    .fetch_optional(pool)
    .await
}

pub async fn delete_session(pool: &SqlitePool, token: &str) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
        .bind(token_hash(token))
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn prune_sessions(pool: &SqlitePool) -> sqlx::Result<u64> {
    Ok(sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
        .bind(time::now_str())
        .execute(pool)
        .await?
        .rows_affected())
}

pub fn session_cookie(token: &str) -> HeaderValue {
    let v = format!(
        "{COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        SESSION_DAYS * 24 * 3600
    );
    HeaderValue::from_str(&v).unwrap_or_else(|_| HeaderValue::from_static(""))
}

pub fn clear_cookie() -> HeaderValue {
    HeaderValue::from_static("librisync_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0")
}

pub fn cookie_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == COOKIE)
        .map(|(_, v)| v.to_string())
        .filter(|v| !v.is_empty())
}

fn api_key_from(headers: &HeaderMap, uri: &Uri) -> Option<String> {
    if let Some(k) = headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
        return Some(k.to_string());
    }
    uri.query().and_then(|q| {
        url::form_urlencoded::parse(q.as_bytes())
            .find(|(k, _)| k == "apikey")
            .map(|(_, v)| v.into_owned())
    })
}

/// Resolve the caller from the API key or session cookie.
pub async fn authenticate(app: &AppState, headers: &HeaderMap, uri: &Uri) -> Option<Principal> {
    if let Some(key) = api_key_from(headers, uri) {
        let expected = app.api_key();
        return constant_time_eq(key.as_bytes(), expected.as_bytes()).then_some(Principal { user: None });
    }
    let token = cookie_token(headers)?;
    match session_user(&app.db, &token).await {
        Ok(Some(user)) => Some(Principal { user: Some(user) }),
        Ok(None) => None,
        Err(e) => {
            tracing::error!("Session lookup failed: {e}");
            None
        }
    }
}

pub fn is_anonymous(path: &str) -> bool {
    !path.starts_with("/api/") || ANONYMOUS.contains(&path)
}

/// Middleware guarding `/api/*`.
pub async fn require_auth(State(app): State<AppState>, mut req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    if is_anonymous(&path) {
        return next.run(req).await;
    }
    match authenticate(&app, req.headers(), req.uri()).await {
        Some(principal) => {
            req.extensions_mut().insert(principal);
            next.run(req).await
        }
        None => ApiError::unauthorized().into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_hash_and_verify() {
        let h = hash_password("correct horse").unwrap();
        assert!(verify_password("correct horse", &h));
        assert!(!verify_password("wrong horse", &h));
        assert!(!verify_password("x", "not a hash"));
    }

    #[test]
    fn constant_time_comparison() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn reads_the_session_cookie_among_others() {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, HeaderValue::from_static("a=1; librisync_session=tok; b=2"));
        assert_eq!(cookie_token(&h).as_deref(), Some("tok"));
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, HeaderValue::from_static("librisync_session="));
        assert_eq!(cookie_token(&h), None);
    }

    #[test]
    fn api_key_from_header_or_query() {
        let mut h = HeaderMap::new();
        let uri: Uri = "/api/v1/events?apikey=abc%20d".parse().unwrap();
        assert_eq!(api_key_from(&h, &uri).as_deref(), Some("abc d"));
        h.insert("x-api-key", HeaderValue::from_static("hdr"));
        assert_eq!(api_key_from(&h, &uri).as_deref(), Some("hdr"));
    }

    #[test]
    fn anonymous_paths() {
        assert!(is_anonymous("/ping"));
        assert!(is_anonymous("/"));
        assert!(is_anonymous("/assets/app.js"));
        assert!(is_anonymous("/api/v1/auth/state"));
        assert!(!is_anonymous("/api/v1/auth/logout"));
        assert!(!is_anonymous("/api/v1/books"));
    }
}
