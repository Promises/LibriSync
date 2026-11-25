use axum::extract::{Path, State};
use axum::response::Response;
use axum::Json;
use serde::{Deserialize, Serialize};

use super::no_content;
use crate::error::{ApiError, ApiResult, JsonBody};
use crate::notify::{self, AbsLibrary, Kind, Message, Notification, MASK};
use crate::state::AppState;

fn masked(mut n: Notification) -> Notification {
    notify::mask(n.kind, &mut n.config);
    n
}

pub async fn list(State(app): State<AppState>) -> ApiResult<Json<Vec<Notification>>> {
    Ok(Json(notify::list(&app.db).await?.into_iter().map(masked).collect()))
}

pub async fn create(State(app): State<AppState>, JsonBody(mut n): JsonBody<Notification>) -> ApiResult<Json<Notification>> {
    notify::validate(&mut n)?;
    let id = notify::insert(&app.db, &n).await?;
    let saved = notify::get(&app.db, id).await?.ok_or_else(|| ApiError::internal("notification vanished"))?;
    Ok(Json(masked(saved)))
}

/// Restore masked secrets from the stored copy when the kind is unchanged.
async fn restore_secrets(app: &AppState, n: &mut Notification) -> ApiResult<()> {
    if n.id <= 0 {
        return Ok(());
    }
    if let Some(stored) = notify::get(&app.db, n.id).await? {
        if stored.kind == n.kind {
            notify::unmask(n.kind, &mut n.config, &stored.config);
        }
    }
    Ok(())
}

pub async fn update(
    State(app): State<AppState>,
    Path(id): Path<i64>,
    JsonBody(mut n): JsonBody<Notification>,
) -> ApiResult<Json<Notification>> {
    if notify::get(&app.db, id).await?.is_none() {
        return Err(ApiError::not_found("Notification not found"));
    }
    n.id = id;
    restore_secrets(&app, &mut n).await?;
    notify::validate(&mut n)?;
    notify::update(&app.db, &n).await?;
    let saved = notify::get(&app.db, id).await?.ok_or_else(|| ApiError::not_found("Notification not found"))?;
    Ok(Json(masked(saved)))
}

pub async fn delete(State(app): State<AppState>, Path(id): Path<i64>) -> ApiResult<Response> {
    if notify::delete(&app.db, id).await? {
        Ok(no_content())
    } else {
        Err(ApiError::not_found("Notification not found"))
    }
}

#[derive(Serialize)]
pub struct TestResult {
    ok: bool,
    message: String,
}

pub async fn test(State(app): State<AppState>, JsonBody(mut n): JsonBody<Notification>) -> ApiResult<Json<TestResult>> {
    restore_secrets(&app, &mut n).await?;
    notify::validate(&mut n)?;
    let msg = Message {
        event_type: "test".into(),
        message: "Test notification from LibriSync".into(),
        account: None,
        book: None,
    };
    let result = app.notifier.send(&app, &n, &msg).await;
    Ok(Json(match result {
        Ok(m) => TestResult { ok: true, message: m },
        Err(e) => TestResult { ok: false, message: e },
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AbsBody {
    url: String,
    api_key: String,
}

pub async fn abs_libraries(State(app): State<AppState>, JsonBody(b): JsonBody<AbsBody>) -> ApiResult<Json<Vec<AbsLibrary>>> {
    let key = if b.api_key == MASK {
        // The UI is editing a saved connection whose key it only has masked.
        notify::list(&app.db)
            .await?
            .into_iter()
            .filter(|n| n.kind == Kind::Audiobookshelf)
            .find(|n| n.config.get("url").and_then(|u| u.as_str()) == Some(b.url.as_str()))
            .and_then(|n| n.config.get("apiKey").and_then(|k| k.as_str()).map(String::from))
            .ok_or_else(|| ApiError::invalid_field("apiKey", "enter the API key"))?
    } else {
        b.api_key
    };
    if url::Url::parse(&b.url).map(|u| !matches!(u.scheme(), "http" | "https")).unwrap_or(true) {
        return Err(ApiError::invalid_field("url", "must be an http(s) URL"));
    }
    notify::abs_libraries(&app.http, &b.url, &key)
        .await
        .map(Json)
        .map_err(ApiError::provider)
}
