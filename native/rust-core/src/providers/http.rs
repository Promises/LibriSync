//! Shared HTTP client helpers for DRM-free providers (Libro.fm, future stores).
//!
//! Kept separate from `AudibleClient`, which is coupled to Audible's domains and
//! `Identity`. Stores commonly sit behind a WAF that inspects the `User-Agent`
//! (and sometimes app-version headers), so the UA is always caller-supplied
//! rather than baked in — see `librofm.rs` for a client that must impersonate the
//! store's own mobile app.

use std::time::Duration;

use reqwest::{Client, ClientBuilder};

use crate::{LibationError, Result};

/// Browser-like UA, for stores whose WAF expects a browser.
pub const BROWSER_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36";

/// A client builder with a sane timeout and the given User-Agent. Callers may
/// layer on `default_headers` before calling [`build`].
pub fn builder(user_agent: &str) -> ClientBuilder {
    Client::builder()
        .user_agent(user_agent)
        .timeout(Duration::from_secs(60))
}

/// Finish a builder, mapping the build error into a `LibationError`.
pub fn build(builder: ClientBuilder) -> Result<Client> {
    builder.build().map_err(|e| LibationError::ApiRequestFailed {
        message: format!("HTTP client build failed: {e}"),
        status_code: None,
        endpoint: None,
    })
}

/// Convenience: a finished client with the given User-Agent.
pub fn client(user_agent: &str) -> Result<Client> {
    build(builder(user_agent))
}
