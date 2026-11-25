//! HTTP API (server/API.md).

mod accounts;
mod auth_routes;
mod books;
mod notifications;
mod queue;
mod settings;
mod system;

use std::any::Any;

use axum::body::Bytes;
use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{middleware, Json, Router};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;

use crate::auth;
use crate::error::ApiError;
use crate::state::AppState;

pub fn router(app: AppState) -> Router {
    let api = Router::new()
        // System
        .route("/system/status", get(system::status))
        .route("/health", get(system::health))
        .route("/system/tasks", get(system::tasks))
        .route("/system/tasks/{name}/run", post(system::run_task))
        .route("/log", get(system::log))
        .route("/events", get(system::events))
        // Auth
        .route("/auth/state", get(auth_routes::state))
        .route("/auth/setup", post(auth_routes::setup))
        .route("/auth/login", post(auth_routes::login))
        .route("/auth/logout", post(auth_routes::logout))
        .route("/auth/password", post(auth_routes::password))
        // Providers and accounts
        .route("/providers", get(accounts::providers))
        .route("/accounts", get(accounts::list))
        .route("/accounts/{id}", get(accounts::get).patch(accounts::patch).delete(accounts::delete))
        .route("/accounts/{id}/sync", post(accounts::sync))
        .route("/accounts/{id}/clear-cooldown", post(accounts::clear_cooldown))
        .route("/login/audible/device-code", post(accounts::device_code_start))
        .route(
            "/login/audible/device-code/{session}",
            get(accounts::device_code_get).delete(accounts::device_code_cancel),
        )
        .route("/login/audible/external", post(accounts::external_start))
        .route("/login/audible/external/{session}", post(accounts::external_complete))
        .route("/login/credentials", post(accounts::credentials))
        // Books
        .route("/books", get(books::list))
        .route("/books/download", post(books::download_many))
        .route("/books/{id}", get(books::get))
        .route("/books/{id}/download", post(books::download))
        .route("/books/{id}/skip", post(books::skip))
        .route("/books/{id}/unskip", post(books::unskip))
        .route("/books/{id}/forget", post(books::forget))
        .route("/books/{id}/cover", get(books::cover))
        .route("/wanted", get(books::wanted))
        // Queue and history
        .route("/queue", get(queue::list))
        .route("/queue/{id}", axum::routing::delete(queue::cancel))
        .route("/queue/{id}/retry", post(queue::retry))
        .route("/queue/{id}/move", post(queue::move_item))
        .route("/history", get(queue::history))
        // Settings
        .route("/settings", get(settings::get).put(settings::put))
        .route("/settings/api-key/regenerate", post(settings::regenerate_api_key))
        // Notifications
        .route("/notifications", get(notifications::list).post(notifications::create))
        .route("/notifications/test", post(notifications::test))
        .route("/notifications/abs/libraries", post(notifications::abs_libraries))
        .route(
            "/notifications/{id}",
            axum::routing::put(notifications::update).delete(notifications::delete),
        );

    Router::new()
        .route("/ping", get(ping).head(ping))
        .nest("/api/v1", api)
        .fallback(crate::web::serve)
        .layer(middleware::from_fn_with_state(app.clone(), auth::require_auth))
        .layer(CompressionLayer::new())
        .layer(CatchPanicLayer::custom(panic_response))
        .layer(TraceLayer::new_for_http())
        .with_state(app)
}

async fn ping() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "OK" }))
}

fn panic_response(_err: Box<dyn Any + Send + 'static>) -> Response {
    ApiError::internal("The request failed unexpectedly").into_response()
}

pub fn no_content() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

/// `page` / `pageSize` query parameters.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Paging {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

impl Paging {
    pub fn resolve(&self) -> Result<(i64, i64), ApiError> {
        let page = self.page.unwrap_or(1);
        let size = self.page_size.unwrap_or(50);
        if page < 1 {
            return Err(ApiError::invalid_field("page", "must be 1 or more"));
        }
        if !(1..=500).contains(&size) {
            return Err(ApiError::invalid_field("pageSize", "must be between 1 and 500"));
        }
        Ok((page, size))
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
    pub items: Vec<T>,
}

/// Split a comma list query value.
pub fn comma_list(v: Option<&str>) -> Vec<String> {
    v.map(|s| s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect())
        .unwrap_or_default()
}

/// A JSON body that may be absent: an empty body means `T::default()`.
pub struct OptionalJson<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned + Default> FromRequest<S> for OptionalJson<T> {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = Bytes::from_request(req, state)
            .await
            .map_err(|e| ApiError::invalid(e.body_text()))?;
        if bytes.iter().all(|b| b.is_ascii_whitespace()) {
            return Ok(Self(T::default()));
        }
        serde_json::from_slice(&bytes)
            .map(Self)
            .map_err(|e| ApiError::invalid(format!("Invalid JSON body: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use crate::test_support::test_app;

    async fn call(app: &AppState, req: HttpRequest<Body>) -> (StatusCode, serde_json::Value, axum::http::HeaderMap) {
        let resp = router(app.clone()).oneshot(req).await.unwrap();
        let status = resp.status();
        let headers = resp.headers().clone();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
        (status, json, headers)
    }

    fn get_req(uri: &str) -> HttpRequest<Body> {
        HttpRequest::get(uri).body(Body::empty()).unwrap()
    }

    fn json_req(method: &str, uri: &str, body: serde_json::Value) -> HttpRequest<Body> {
        HttpRequest::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn ping_and_auth_state_are_anonymous_everything_else_is_not() {
        let (app, _dir) = test_app().await;
        let (s, j, _) = call(&app, get_req("/ping")).await;
        assert_eq!((s, j["status"].as_str()), (StatusCode::OK, Some("OK")));

        let (s, j, _) = call(&app, get_req("/api/v1/auth/state")).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(j["setupRequired"], true);
        assert_eq!(j["authenticated"], false);

        let (s, j, _) = call(&app, get_req("/api/v1/books")).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        assert_eq!(j["error"], "unauthorized");

        let (s, _, _) = call(&app, get_req("/api/v1/nope")).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "unknown API paths don't leak existence");
    }

    #[tokio::test]
    async fn api_key_works_in_header_and_query() {
        let (app, _dir) = test_app().await;
        let key = app.api_key();
        let req = HttpRequest::get("/api/v1/accounts").header("X-Api-Key", &key).body(Body::empty()).unwrap();
        let (s, j, _) = call(&app, req).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(j, serde_json::json!([]));

        let (s, _, _) = call(&app, get_req(&format!("/api/v1/books?apikey={key}"))).await;
        assert_eq!(s, StatusCode::OK);

        let req = HttpRequest::get("/api/v1/accounts").header("X-Api-Key", "wrong").body(Body::empty()).unwrap();
        let (s, _, _) = call(&app, req).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn setup_login_session_and_logout() {
        let (app, _dir) = test_app().await;
        let (s, j, _) = call(&app, json_req("POST", "/api/v1/auth/setup", serde_json::json!({"username": "admin", "password": "short"}))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(j["error"], "invalid_input");

        let (s, _, h) = call(&app, json_req("POST", "/api/v1/auth/setup", serde_json::json!({"username": "admin", "password": "long enough"}))).await;
        assert_eq!(s, StatusCode::NO_CONTENT);
        let cookie = h.get("set-cookie").unwrap().to_str().unwrap().to_string();
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax"));
        let session = cookie.split(';').next().unwrap().to_string();

        // Setup is one-shot.
        let (s, j, _) = call(&app, json_req("POST", "/api/v1/auth/setup", serde_json::json!({"username": "x", "password": "long enough"}))).await;
        assert_eq!((s, j["error"].as_str()), (StatusCode::CONFLICT, Some("conflict")));

        let (s, _, _) = call(&app, json_req("POST", "/api/v1/auth/login", serde_json::json!({"username": "admin", "password": "wrong password"}))).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);

        let req = HttpRequest::get("/api/v1/auth/state").header("cookie", &session).body(Body::empty()).unwrap();
        let (_, j, _) = call(&app, req).await;
        assert_eq!(j["authenticated"], true);
        assert_eq!(j["username"], "admin");

        let req = HttpRequest::post("/api/v1/auth/logout").header("cookie", &session).body(Body::empty()).unwrap();
        let (s, _, _) = call(&app, req).await;
        assert_eq!(s, StatusCode::NO_CONTENT);
        let req = HttpRequest::get("/api/v1/books").header("cookie", &session).body(Body::empty()).unwrap();
        let (s, _, _) = call(&app, req).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "session ended");
    }

    #[tokio::test]
    async fn settings_round_trip_and_validation() {
        let (app, _dir) = test_app().await;
        let key = app.api_key();
        let req = HttpRequest::get("/api/v1/settings").header("X-Api-Key", &key).body(Body::empty()).unwrap();
        let (s, mut settings, _) = call(&app, req).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(settings["general"]["apiKey"], key.as_str());

        settings["downloads"]["concurrency"] = serde_json::json!(2);
        settings["general"]["apiKey"] = serde_json::json!("ignored");
        let mut req = json_req("PUT", "/api/v1/settings", settings.clone());
        req.headers_mut().insert("x-api-key", key.parse().unwrap());
        let (s, j, _) = call(&app, req).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(j["downloads"]["concurrency"], 2);
        assert_eq!(j["general"]["apiKey"], key.as_str(), "the API key is read-only");

        settings["sync"]["intervalHours"] = serde_json::json!(0);
        let mut req = json_req("PUT", "/api/v1/settings", settings);
        req.headers_mut().insert("x-api-key", key.parse().unwrap());
        let (s, j, _) = call(&app, req).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(j["error"], "invalid_input");
        assert!(j["message"].as_str().unwrap().contains("sync.intervalHours"));
    }
}
