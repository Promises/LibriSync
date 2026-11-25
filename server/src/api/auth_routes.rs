use std::time::Duration;

use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::json;

use crate::auth::{self, Principal};
use crate::error::{ApiError, ApiResult, JsonBody};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct Credentials {
    username: String,
    password: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasswordChange {
    current_password: String,
    new_password: String,
}

pub async fn state(State(app): State<AppState>, headers: HeaderMap, uri: Uri) -> ApiResult<Json<serde_json::Value>> {
    let setup_required = auth::user_count(&app.db).await? == 0;
    let principal = auth::authenticate(&app, &headers, &uri).await;
    let username = match &principal {
        Some(Principal { user: Some((_, name)) }) => Some(name.clone()),
        Some(Principal { user: None }) => auth::first_user(&app.db).await?.map(|u| u.1),
        None => None,
    };
    Ok(Json(json!({
        "setupRequired": setup_required,
        "authenticated": principal.is_some(),
        "username": username,
    })))
}

fn with_session(token: &str) -> Response {
    (StatusCode::NO_CONTENT, [(header::SET_COOKIE, auth::session_cookie(token))]).into_response()
}

pub async fn setup(State(app): State<AppState>, JsonBody(c): JsonBody<Credentials>) -> ApiResult<Response> {
    let username = c.username.trim();
    if username.is_empty() || username.chars().count() > 64 {
        return Err(ApiError::invalid_field("username", "must be 1–64 characters"));
    }
    auth::check_new_password(&c.password)?;
    let Some(user_id) = auth::create_first_user(&app.db, username, &c.password).await? else {
        return Err(ApiError::conflict("Setup is already complete"));
    };
    tracing::info!("Created the admin user");
    let token = auth::create_session(&app.db, user_id).await?;
    Ok(with_session(&token))
}

pub async fn login(State(app): State<AppState>, JsonBody(c): JsonBody<Credentials>) -> ApiResult<Response> {
    if auth::user_count(&app.db).await? == 0 {
        return Err(ApiError::setup_required());
    }
    let user = auth::find_user(&app.db, c.username.trim()).await?;
    let ok = match &user {
        Some((_, _, hash)) => auth::verify_password_limited(&c.password, hash).await,
        // Hash anyway so an unknown username takes as long as a wrong password.
        None => {
            let _ = auth::hash_password_limited(&c.password).await;
            false
        }
    };
    match (ok, user) {
        (true, Some((id, _, _))) => {
            let token = auth::create_session(&app.db, id).await?;
            Ok(with_session(&token))
        }
        _ => {
            tracing::warn!("Failed sign-in attempt");
            // Slow down guessing.
            tokio::time::sleep(Duration::from_millis(750)).await;
            Err(ApiError::bad_credentials())
        }
    }
}

pub async fn logout(State(app): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    if let Some(token) = auth::cookie_token(&headers) {
        auth::delete_session(&app.db, &token).await?;
    }
    Ok((StatusCode::NO_CONTENT, [(header::SET_COOKIE, auth::clear_cookie())]).into_response())
}

pub async fn password(
    State(app): State<AppState>,
    Extension(principal): Extension<Principal>,
    JsonBody(body): JsonBody<PasswordChange>,
) -> ApiResult<Response> {
    let user = match principal.user {
        Some((id, _)) => sqlx::query_as::<_, (i64, String, String)>("SELECT id, username, password_hash FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(&app.db)
            .await?,
        None => auth::first_user(&app.db).await?,
    };
    let Some((id, _, hash)) = user else { return Err(ApiError::setup_required()) };
    if !auth::verify_password_limited(&body.current_password, &hash).await {
        return Err(ApiError::invalid_field("currentPassword", "is not correct"));
    }
    auth::check_new_password(&body.new_password).map_err(|_| {
        ApiError::invalid_field("newPassword", format!("must be at least {} characters", auth::MIN_PASSWORD))
    })?;
    auth::set_password(&app.db, id, &body.new_password).await?;
    // The caller's own session ended with the others; start a fresh one.
    let token = auth::create_session(&app.db, id).await?;
    Ok(with_session(&token))
}
