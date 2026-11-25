//! Cover proxy with an on-disk cache in `<config>/cache/covers`, so the UI shows
//! covers without reaching the stores' CDNs on every page view.

use std::path::PathBuf;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::error::ApiError;
use crate::state::App;

const MAX_BYTES: usize = 10 * 1024 * 1024;

/// The image type from its leading bytes.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(b"GIF8") {
        Some("image/gif")
    } else {
        None
    }
}

fn cache_path(app: &App, book_id: &str) -> PathBuf {
    app.cfg.cover_cache_dir().join(hex::encode(Sha256::digest(book_id.as_bytes())))
}

/// The cover for `book_id` as `(bytes, content type)`.
pub async fn get(app: &App, book_id: &str) -> Result<(Vec<u8>, &'static str), ApiError> {
    let path = cache_path(app, book_id);
    if let Ok(bytes) = tokio::fs::read(&path).await {
        if let Some(ct) = sniff(&bytes) {
            return Ok((bytes, ct));
        }
    }

    let url: Option<Option<String>> = sqlx::query_scalar(
        "SELECT COALESCE(NULLIF(picture_large, ''), NULLIF(picture_id, '')) FROM core.Books WHERE audible_product_id = ?",
    )
    .bind(book_id)
    .fetch_optional(&app.db)
    .await?;
    let url = url
        .ok_or_else(|| ApiError::not_found("Book not found"))?
        .ok_or_else(|| ApiError::not_found("This book has no cover"))?;
    let url = match url.strip_prefix("//") {
        Some(rest) => format!("https://{rest}"),
        None => url,
    };
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(ApiError::not_found("This book has no usable cover URL"));
    }

    let resp = app
        .http
        .get(&url)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| ApiError::provider(format!("Could not fetch the cover: {e}")))?;
    if !resp.status().is_success() {
        return Err(ApiError::provider(format!("The cover server answered HTTP {}", resp.status().as_u16())));
    }
    if resp.content_length().is_some_and(|l| l as usize > MAX_BYTES) {
        return Err(ApiError::provider("The cover image is too large"));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| ApiError::provider(format!("Could not fetch the cover: {e}")))?;
    if bytes.len() > MAX_BYTES {
        return Err(ApiError::provider("The cover image is too large"));
    }
    let ct = sniff(&bytes).ok_or_else(|| ApiError::provider("The cover URL did not return an image"))?;

    // Cache best-effort: a full disk must not break the response.
    if let Some(dir) = path.parent() {
        let tmp = path.with_extension("tmp");
        let written = async {
            tokio::fs::create_dir_all(dir).await?;
            tokio::fs::write(&tmp, &bytes).await?;
            tokio::fs::rename(&tmp, &path).await
        }
        .await;
        if let Err(e) = written {
            tracing::debug!("Could not cache a cover: {e}");
        }
    }
    Ok((bytes.to_vec(), ct))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_common_image_types() {
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n...."), Some("image/png"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff(b"<html>"), None);
    }
}
