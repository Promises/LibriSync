use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rust_core::api::auth::Locale;
use rust_core::providers::{AnyProvider, ProviderId};
use rust_core::LibationError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::no_content;
use crate::accounts::{self, AccountView};
use crate::error::{ApiError, ApiResult, JsonBody, QueryParams};
use crate::history::{self, NewEvent};
use crate::login::{self, DeviceCodeSession};
use crate::queue;
use crate::state::AppState;

#[derive(Serialize)]
pub struct LocaleInfo {
    code: String,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    id: &'static str,
    name: &'static str,
    login_methods: Vec<&'static str>,
    locales: Vec<LocaleInfo>,
    formats: Vec<&'static str>,
}

pub async fn providers() -> Json<Vec<ProviderInfo>> {
    let locales = Locale::all()
        .into_iter()
        .map(|l| LocaleInfo { code: l.country_code, name: l.name })
        .collect();
    Json(vec![
        ProviderInfo {
            id: "audible",
            name: "Audible",
            login_methods: vec!["deviceCode", "externalBrowser"],
            locales,
            formats: vec!["m4b", "mp3"],
        },
        ProviderInfo {
            id: "librofm",
            name: "Libro.fm",
            login_methods: vec!["credentials"],
            locales: Vec::new(),
            formats: vec!["m4b", "mp3"],
        },
        ProviderInfo {
            id: "chirp",
            name: "Chirp",
            login_methods: vec!["credentials"],
            locales: Vec::new(),
            formats: vec!["m4b", "mp3"],
        },
    ])
}

pub async fn list(State(app): State<AppState>) -> ApiResult<Json<Vec<AccountView>>> {
    Ok(Json(accounts::list(&app).await?))
}

async fn view_or_404(app: &AppState, id: &str) -> ApiResult<AccountView> {
    accounts::get(app, id).await?.ok_or_else(|| ApiError::not_found("Account not found"))
}

pub async fn get(State(app): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<AccountView>> {
    Ok(Json(view_or_404(&app, &id).await?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountPatch {
    enabled: Option<bool>,
    auto_download_new: Option<bool>,
    backfill: Option<bool>,
}

pub async fn patch(
    State(app): State<AppState>,
    Path(id): Path<String>,
    JsonBody(p): JsonBody<AccountPatch>,
) -> ApiResult<Json<AccountView>> {
    view_or_404(&app, &id).await?;
    accounts::ensure_state(&app, &id).await?;
    sqlx::query(
        "UPDATE account_state SET enabled = COALESCE(?, enabled), auto_download_new = COALESCE(?, auto_download_new),
                backfill = COALESCE(?, backfill) WHERE account_id = ?",
    )
    .bind(p.enabled)
    .bind(p.auto_download_new)
    .bind(p.backfill)
    .bind(&id)
    .execute(&app.db)
    .await?;
    if p.backfill == Some(false) || p.enabled == Some(false) {
        // Turning back-fill off takes the account's not-yet-started back-fill items out.
        let removed: Vec<i64> = sqlx::query_scalar(
            "DELETE FROM queue WHERE account_id = ? AND priority = ? AND state IN ('queued','waiting') RETURNING id",
        )
        .bind(&id)
        .bind(queue::Priority::Backfill.as_i64())
        .fetch_all(&app.db)
        .await?;
        for qid in removed {
            app.hub.publish("queueRemoved", &json!({ "id": qid.to_string() }));
        }
    }
    let view = view_or_404(&app, &id).await?;
    app.hub.publish("account", &view);
    app.queue.wake();
    Ok(Json(view))
}

#[derive(Deserialize)]
pub struct DeleteQuery {
    deregister: Option<bool>,
}

pub async fn delete(
    State(app): State<AppState>,
    Path(id): Path<String>,
    QueryParams(q): QueryParams<DeleteQuery>,
) -> ApiResult<Response> {
    let account = accounts::load_json(&app, &id).await?.ok_or_else(|| ApiError::not_found("Account not found"))?;
    let provider = accounts::provider_of(&account)?;
    let name = account.get("account_name").and_then(Value::as_str).unwrap_or(&id).to_string();

    if q.deregister.unwrap_or(false) && provider == ProviderId::Audible {
        let parsed: rust_core::api::auth::Account = serde_json::from_value(account.clone())
            .map_err(|e| ApiError::internal(format!("Stored account is unreadable: {e}")))?;
        rust_core::api::auth::deregister_device(&parsed)
            .await
            .map_err(|e| ApiError::provider(format!("Could not deregister the device: {e}")))?;
    }

    // Stop its work: running items are cancelled, queued ones dropped.
    let running: Vec<i64> = sqlx::query_scalar("SELECT id FROM queue WHERE account_id = ? AND state IN ('planning','running')")
        .bind(&id)
        .fetch_all(&app.db)
        .await?;
    for qid in running {
        app.queue.cancel(qid);
    }
    let removed: Vec<i64> =
        sqlx::query_scalar("DELETE FROM queue WHERE account_id = ? AND state IN ('queued','waiting','failed') RETURNING id")
            .bind(&id)
            .fetch_all(&app.db)
            .await?;
    for qid in removed {
        app.hub.publish("queueRemoved", &json!({ "id": qid.to_string() }));
    }

    rust_core::storage::accounts::delete_account(app.core.pool(), &id).await?;
    // Licence pacing (cooldown, request log) is kept: it belongs to the Amazon customer,
    // so removing and re-adding the account must not forget a throttle.
    sqlx::query("DELETE FROM account_state WHERE account_id = ?").bind(&id).execute(&app.db).await?;
    history::record(
        &app,
        NewEvent::new("accountRemoved", format!("Removed {} account {name}", provider.as_str())).account(&id),
    )
    .await;
    Ok(no_content())
}

pub async fn sync(State(app): State<AppState>, Path(id): Path<String>) -> ApiResult<Response> {
    view_or_404(&app, &id).await?;
    crate::sync::spawn(app.clone(), id);
    Ok(StatusCode::ACCEPTED.into_response())
}

pub async fn clear_cooldown(State(app): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<AccountView>> {
    view_or_404(&app, &id).await?;
    accounts::clear_cooldown(&app, &id).await?;
    let view = view_or_404(&app, &id).await?;
    app.hub.publish("account", &view);
    Ok(Json(view))
}

#[derive(Deserialize)]
pub struct LocaleBody {
    locale: String,
}

pub async fn device_code_start(State(app): State<AppState>, JsonBody(b): JsonBody<LocaleBody>) -> ApiResult<Json<DeviceCodeSession>> {
    Ok(Json(login::start_device_code(&app, &b.locale).await?))
}

pub async fn device_code_get(State(app): State<AppState>, Path(session): Path<String>) -> ApiResult<Json<DeviceCodeSession>> {
    app.logins
        .device_view(&session)
        .map(Json)
        .ok_or_else(|| ApiError::not_found("Sign-in session not found or expired"))
}

pub async fn device_code_cancel(State(app): State<AppState>, Path(session): Path<String>) -> ApiResult<Response> {
    if app.logins.cancel_device(&session) {
        Ok(no_content())
    } else {
        Err(ApiError::not_found("Sign-in session not found or expired"))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalStart {
    session_id: String,
    url: String,
}

pub async fn external_start(State(app): State<AppState>, JsonBody(b): JsonBody<LocaleBody>) -> ApiResult<Json<ExternalStart>> {
    let (session_id, url) = login::start_external(&app, &b.locale)?;
    Ok(Json(ExternalStart { session_id, url }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalComplete {
    response_url: String,
}

pub async fn external_complete(
    State(app): State<AppState>,
    Path(session): Path<String>,
    JsonBody(b): JsonBody<ExternalComplete>,
) -> ApiResult<Json<AccountView>> {
    Ok(Json(login::complete_external(&app, &session, &b.response_url).await?))
}

#[derive(Deserialize)]
pub struct CredentialLogin {
    provider: String,
    email: String,
    password: String,
}

/// Email + password sign-in for DRM-free stores — the server's `CredentialLoginScreen`.
pub async fn credentials(State(app): State<AppState>, JsonBody(b): JsonBody<CredentialLogin>) -> ApiResult<Json<AccountView>> {
    let provider = match b.provider.as_str() {
        "librofm" => ProviderId::Librofm,
        "chirp" => ProviderId::Chirp,
        _ => return Err(ApiError::invalid_field("provider", "must be librofm or chirp")),
    };
    let email = b.email.trim().to_string();
    if email.is_empty() {
        return Err(ApiError::invalid_field("email", "must not be empty"));
    }
    if b.password.is_empty() {
        return Err(ApiError::invalid_field("password", "must not be empty"));
    }
    let p = AnyProvider::get(provider)?;
    let creds = p
        .as_provider()
        .login(json!({ "username": email, "password": b.password }))
        .await
        // From a login call, `auth_expired` means the store refused these credentials.
        .map_err(|e| match e {
            LibationError::AuthenticationFailed { message, .. } => ApiError::provider_rejected(message),
            other if other.error_code() == "auth_expired" => ApiError::provider_rejected(other.to_string()),
            other => ApiError::provider(format!("Sign-in failed: {other}")),
        })?;
    // A provider may namespace its account id (Chirp: `chirp:<user id>`) so the same
    // email at two stores can't collide; otherwise the email is the id.
    let account_id = creds
        .get("account_id")
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or_else(|| email.clone());
    let account = json!({
        "account_id": account_id,
        "account_name": email,
        "provider": provider.as_str(),
        "identity": creds,
    });
    Ok(Json(accounts::add(&app, &account_id, &account).await?))
}
