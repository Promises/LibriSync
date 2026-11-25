//! API errors: every non-2xx response is `{"error": "<code>", "message": "<text>"}`.

use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::de::DeserializeOwned;
use serde_json::json;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    /// The offending field for `invalid_input`, when known.
    pub field: Option<String>,
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into(), field: None }
    }
    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", "Authentication required")
    }
    pub fn bad_credentials() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", "Invalid username or password")
    }
    pub fn not_found(what: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", what)
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_input", message)
    }
    pub fn invalid_field(field: &str, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_input",
            message: format!("{field}: {message}"),
            field: Some(field.to_string()),
        }
    }
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }
    pub fn setup_required() -> Self {
        Self::new(StatusCode::CONFLICT, "setup_required", "Create the admin user first")
    }
    pub fn account_cooldown(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "account_cooldown", message)
    }
    pub fn provider(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, "provider_error", message)
    }
    /// A provider refused the user's input (wrong password, bad pasted URL, …).
    pub fn provider_rejected(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "provider_error", message)
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if self.status.is_server_error() {
            tracing::error!(code = self.code, "{}", self.message);
        }
        let mut body = json!({ "error": self.code, "message": self.message });
        if let Some(field) = self.field {
            body["field"] = json!(field);
        }
        (self.status, Json(body)).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        Self::internal(format!("Database error: {e}"))
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        Self::internal(format!("{e:#}"))
    }
}

impl From<rust_core::LibationError> for ApiError {
    fn from(e: rust_core::LibationError) -> Self {
        Self::internal(e.to_string())
    }
}

/// `Json<T>` whose rejections come back in the API's error shape (`invalid_input`).
pub struct JsonBody<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for JsonBody<T> {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(v)) => Ok(Self(v)),
            Err(rejection) => Err(json_rejection(rejection)),
        }
    }
}

fn json_rejection(r: JsonRejection) -> ApiError {
    ApiError::invalid(r.body_text())
}

/// `Query<T>` with API-shaped rejections.
pub struct QueryParams<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequestParts<S> for QueryParams<T> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Query::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Query(v)) => Ok(Self(v)),
            Err(r) => Err(query_rejection(r)),
        }
    }
}

fn query_rejection(r: QueryRejection) -> ApiError {
    ApiError::invalid(r.body_text())
}
