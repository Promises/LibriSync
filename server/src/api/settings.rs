use axum::extract::State;
use axum::Json;
use serde_json::json;

use crate::error::{ApiError, ApiResult, JsonBody};
use crate::settings::{self, Settings};
use crate::state::AppState;

pub async fn get(State(app): State<AppState>) -> Json<Settings> {
    Json(app.settings())
}

pub async fn put(State(app): State<AppState>, JsonBody(new): JsonBody<Settings>) -> ApiResult<Json<Settings>> {
    new.validate().map_err(|e| ApiError::invalid_field(e.field, e.message))?;
    let old = app.settings();
    settings::save(&app.db, &new).await?;
    app.set_settings(new.clone());
    if old.general.log_level != new.general.log_level {
        (app.set_log_level)(new.general.log_level.as_str());
    }
    tracing::info!("Settings updated");
    // Concurrency, pacing or the window may have changed.
    app.queue.wake();
    Ok(Json(app.settings()))
}

pub async fn regenerate_api_key(State(app): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    let key = settings::generate_api_key();
    settings::store_api_key(&app.db, &key).await?;
    app.set_api_key(key.clone());
    tracing::info!("API key regenerated");
    Ok(Json(json!({ "apiKey": key })))
}
