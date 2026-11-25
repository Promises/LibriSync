//! The web UI, embedded from `server/web/dist` at build time. Unknown non-API paths
//! fall back to `index.html` so client-side routes work on reload.
//!
//! The build must succeed before the UI exists (`allow_missing`); the server then
//! answers UI routes with a short notice instead.

use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

use crate::error::ApiError;

#[derive(RustEmbed)]
#[folder = "web/dist"]
#[allow_missing = true]
struct Assets;

fn file_response(path: &str) -> Option<Response> {
    let file = Assets::get(path)?;
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    // Vite fingerprints everything under assets/; index.html must always revalidate.
    let cache = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    Some(
        (
            [(header::CONTENT_TYPE, mime.as_ref().to_string()), (header::CACHE_CONTROL, cache.to_string())],
            file.data.into_owned(),
        )
            .into_response(),
    )
}

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path == "api" || path.starts_with("api/") {
        return ApiError::not_found("No such API endpoint").into_response();
    }
    let path = if path.is_empty() { "index.html" } else { path };
    if let Some(r) = file_response(path) {
        return r;
    }
    // A missing file with an extension is a real 404, not a client-side route.
    let last = path.rsplit('/').next().unwrap_or_default();
    if last.contains('.') {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    file_response("index.html").unwrap_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            "The LibriSync web UI is not included in this build. The API is available under /api/v1.",
        )
            .into_response()
    })
}
