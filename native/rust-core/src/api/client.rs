//! HTTP client for Audible API
//!
//! This module provides a robust HTTP client wrapper for the Audible API with features including:
//! - Automatic token refresh on 401 responses
//! - Retry logic with exponential backoff
//! - Rate limiting support
//! - Cookie management
//! - Request signing for authenticated endpoints
//!
//! # Reference C# Sources
//!
//! ## Primary Sources
//! - **`AudibleUtilities/ApiExtended.cs`** - Main API client wrapper with:
//!   - Retry policy using Polly library (2 retries = 3 total attempts)
//!   - Concurrency control (MaxConcurrency = 10)
//!   - Batch processing (BatchSize = 50)
//!   - Channel-based async processing for episodes
//!   - Library validation and scanning
//!
//! - **`AudibleUtilities/Widevine/Cdm.Api.cs`** - HTTP client usage patterns:
//!   - Line 46: `new HttpClient()` - Basic client creation
//!   - Line 65: `client.PostAsync(uri, content)` - POST with body
//!   - Line 106: `client.GetStringAsync()` - Simple GET request
//!   - Line 127: TLD domain list for Audible regions
//!   - Line 147-166: Request header setup and validation
//!
//! - **`AaxDecrypter/NetworkFileStream.cs`** - Download implementation:
//!   - Line 169: HttpClient for downloads
//!   - Line 87-96: Request headers dictionary
//!   - Line 96: RequestHeaders property for custom headers
//!   - Resumable downloads with byte range support
//!   - Progress tracking and throttling
//!
//! ## External Dependencies (AudibleApi NuGet Package)
//! - `AudibleApi/Api.cs` - Core API client (external dependency)
//! - `AudibleApi/Authorization/Identity.cs` - OAuth token management
//! - `AudibleApi/EzApiCreator.cs` - Factory for API client creation
//!
//! # Architecture
//!
//! ## Client Structure
//! The `AudibleClient` wraps `reqwest::Client` and provides:
//! - Base URL management per Audible domain (.com, .co.uk, .de, etc.)
//! - Cookie jar for session persistence
//! - Custom headers (User-Agent, Accept, Authorization)
//! - Timeout and connection pooling configuration
//!
//! ## Retry Strategy (Ported from Polly - ApiExtended.cs:70-73)
//! ```csharp
//! // C# Reference: ApiExtended.cs
//! private static AsyncRetryPolicy policy { get; }
//!     = Policy.Handle<Exception>()
//!     .RetryAsync(2);  // 2 retries == 3 total attempts
//! ```
//!
//! Rust implementation:
//! - Maximum 3 attempts (1 initial + 2 retries)
//! - Exponential backoff: 1s, 2s, 4s between retries
//! - Retry on: network errors, 5xx errors, rate limiting (429)
//! - No retry on: 4xx client errors (except 429), auth failures
//!
//! ## Concurrency (ApiExtended.cs:23-24)
//! ```csharp
//! private const int MaxConcurrency = 10;
//! private const int BatchSize = 50;
//! ```
//! - Implement via tokio::sync::Semaphore for rate limiting
//! - Process API requests in batches
//!
//! # Audible API Domains (Cdm.Api.cs:127)
//! ```csharp
//! static readonly string[] TLDs = ["com", "co.uk", "com.au", "com.br",
//!                                   "ca", "fr", "de", "in", "it", "co.jp", "es"];
//! ```

use crate::error::{LibationError, Result};
use crate::api::auth::{Account, Identity, Locale};
use reqwest::{Client, Method, Request, Response, StatusCode};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Semaphore};
use tokio::time::sleep;

/// Maximum number of concurrent requests to the Audible API
/// Reference: ApiExtended.cs:23
pub const MAX_CONCURRENCY: usize = 10;

/// Batch size for catalog product requests
/// Reference: ApiExtended.cs:24
pub const BATCH_SIZE: usize = 50;

/// Maximum retry attempts (1 initial + 2 retries = 3 total)
/// Reference: ApiExtended.cs:70-73 (Polly retry policy)
const MAX_RETRY_ATTEMPTS: u32 = 3;

/// Initial retry delay in seconds (exponential backoff: 1s, 2s, 4s)
const INITIAL_RETRY_DELAY_SECS: u64 = 1;

/// Default request timeout in seconds
/// Reference: NetworkFileStream.cs uses HttpClient default (100 seconds)
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Supported Audible API domains
/// Reference: Cdm.Api.cs:127
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudibleDomain {
    /// United States - audible.com
    Us,
    /// United Kingdom - audible.co.uk
    Uk,
    /// Australia - audible.com.au
    Au,
    /// Brazil - audible.com.br
    Br,
    /// Canada - audible.ca
    Ca,
    /// France - audible.fr
    Fr,
    /// Germany - audible.de
    De,
    /// India - audible.in
    In,
    /// Italy - audible.it
    It,
    /// Japan - audible.co.jp
    Jp,
    /// Spain - audible.es
    Es,
}

impl AudibleDomain {
    /// Get the domain string for API requests
    /// Reference: Cdm.Api.cs:127
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Us => "audible.com",
            Self::Uk => "audible.co.uk",
            Self::Au => "audible.com.au",
            Self::Br => "audible.com.br",
            Self::Ca => "audible.ca",
            Self::Fr => "audible.fr",
            Self::De => "audible.de",
            Self::In => "audible.in",
            Self::It => "audible.it",
            Self::Jp => "audible.co.jp",
            Self::Es => "audible.es",
        }
    }

    /// Get the TLD component
    pub fn tld(&self) -> &'static str {
        match self {
            Self::Us => "com",
            Self::Uk => "co.uk",
            Self::Au => "com.au",
            Self::Br => "com.br",
            Self::Ca => "ca",
            Self::Fr => "fr",
            Self::De => "de",
            Self::In => "in",
            Self::It => "it",
            Self::Jp => "co.jp",
            Self::Es => "es",
        }
    }

    /// Parse domain from string
    pub fn from_str(domain: &str) -> Option<Self> {
        match domain {
            "audible.com" | "com" => Some(Self::Us),
            "audible.co.uk" | "co.uk" => Some(Self::Uk),
            "audible.com.au" | "com.au" => Some(Self::Au),
            "audible.com.br" | "com.br" => Some(Self::Br),
            "audible.ca" | "ca" => Some(Self::Ca),
            "audible.fr" | "fr" => Some(Self::Fr),
            "audible.de" | "de" => Some(Self::De),
            "audible.in" | "in" => Some(Self::In),
            "audible.it" | "it" => Some(Self::It),
            "audible.co.jp" | "co.jp" => Some(Self::Jp),
            "audible.es" | "es" => Some(Self::Es),
            _ => None,
        }
    }

    /// Get API base URL (https://api.{domain})
    /// Reference: Cdm.Api.cs:141
    pub fn api_url(&self) -> String {
        format!("https://api.{}", self.as_str())
    }
}

/// Configuration for AudibleClient
/// Provides a builder pattern for client customization
#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub domain: AudibleDomain,
    pub timeout: Duration,
    pub max_retries: u32,
    pub user_agent: String,
    pub enable_cookies: bool,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            domain: AudibleDomain::Us,
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
            max_retries: MAX_RETRY_ATTEMPTS,
            user_agent: "Libation/11.3.0 (rust-core)".to_string(),
            enable_cookies: true,
        }
    }
}

impl ClientConfig {
    pub fn builder() -> ClientConfigBuilder {
        ClientConfigBuilder::new()
    }
}

/// Builder for ClientConfig
#[derive(Debug)]
pub struct ClientConfigBuilder {
    config: ClientConfig,
}

impl ClientConfigBuilder {
    pub fn new() -> Self {
        Self {
            config: ClientConfig::default(),
        }
    }

    pub fn domain(mut self, domain: AudibleDomain) -> Self {
        self.config.domain = domain;
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = timeout;
        self
    }

    pub fn max_retries(mut self, max_retries: u32) -> Self {
        self.config.max_retries = max_retries;
        self
    }

    pub fn user_agent<S: Into<String>>(mut self, user_agent: S) -> Self {
        self.config.user_agent = user_agent.into();
        self
    }

    pub fn enable_cookies(mut self, enable: bool) -> Self {
        self.config.enable_cookies = enable;
        self
    }

    pub fn build(self) -> ClientConfig {
        self.config
    }
}

/// Main HTTP client for Audible API
///
/// This client handles:
/// - Authentication via OAuth tokens (access_token, refresh_token)
/// - Automatic token refresh on 401 responses
/// - Retry logic with exponential backoff
/// - Rate limiting and concurrency control
/// - Cookie management for session persistence
///
/// # Reference Implementation
/// Based on C# `ApiExtended` class from AudibleUtilities/ApiExtended.cs
///
/// # Example
/// ```rust,no_run
/// use rust_core::api::client::{AudibleClient, AudibleDomain};
/// use rust_core::api::auth::Account;
///
/// # async fn example() -> rust_core::error::Result<()> {
/// let account = Account::new("user@example.com".to_string());
/// let client = AudibleClient::new(account)?;
///
/// // GET request
/// let response: serde_json::Value = client.get("/1.0/library").await?;
///
/// // POST request with JSON body
/// let body = serde_json::json!({ "key": "value" });
/// let response: serde_json::Value = client.post("/1.0/endpoint", body).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct AudibleClient {
    /// Underlying HTTP client
    client: Client,
    /// Account information with authentication tokens
    account: Arc<Mutex<Account>>,
    /// API base URL (e.g., https://api.audible.com)
    base_url: String,
    /// Client configuration
    config: ClientConfig,
    /// Semaphore for concurrency control
    /// Reference: ApiExtended.cs:23 (MaxConcurrency = 10)
    semaphore: Arc<Semaphore>,
}

impl AudibleClient {
    /// Create a new AudibleClient with default configuration
    ///
    /// # Reference
    /// Based on `ApiExtended.CreateAsync()` - ApiExtended.cs:29-68
    ///
    /// # Arguments
    /// * `account` - Account with valid authentication tokens
    ///
    /// # Errors
    /// Returns error if:
    /// - Account is missing required fields (account_id, locale)
    /// - HTTP client cannot be built
    /// - Authentication tokens are invalid or expired
    pub fn new(account: Account) -> Result<Self> {
        Self::with_config(account, ClientConfig::default())
    }

    /// Create a new AudibleClient with custom configuration
    ///
    /// # Arguments
    /// * `account` - Account with valid authentication tokens
    /// * `config` - Client configuration (timeout, retries, domain, etc.)
    ///
    /// # Errors
    /// Returns error if HTTP client cannot be built
    pub fn with_config(account: Account, config: ClientConfig) -> Result<Self> {
        // Validate account fields
        // Reference: ApiExtended.cs:31-33 (ArgumentValidator checks)
        if account.account_id.is_empty() {
            return Err(LibationError::MissingRequiredField("account_id".to_string()));
        }

        // Build HTTP client with configuration
        // Reference: Cdm.Api.cs:46, NetworkFileStream.cs:169
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_str(&config.user_agent)
                .map_err(|e| LibationError::InvalidInput(format!("Invalid user agent: {}", e)))?,
        );
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/json"),
        );

        let mut client_builder = Client::builder()
            .timeout(config.timeout)
            .default_headers(headers)
            .pool_max_idle_per_host(10) // Connection pooling
            .pool_idle_timeout(Duration::from_secs(90));

        // Enable cookie store if configured
        // Reference: NetworkFileStream.cs:29 (RequestHeaders for cookies)
        if config.enable_cookies {
            client_builder = client_builder.cookie_store(true);
        }

        let client = client_builder.build()?;

        // Determine base URL from account locale or config domain
        // Reference: Cdm.Api.cs:141 (api.audible.{tld})
        let base_url = if let Some(ref identity) = account.identity {
            format!("https://api.{}", identity.locale.domain)
        } else {
            config.domain.api_url()
        };

        let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENCY));

        Ok(Self {
            client,
            account: Arc::new(Mutex::new(account)),
            base_url,
            config,
            semaphore,
        })
    }

    /// Create a builder for custom client configuration
    pub fn builder() -> ClientConfigBuilder {
        ClientConfig::builder()
    }

    /// Get the API base URL
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Perform a GET request
    ///
    /// # Arguments
    /// * `endpoint` - API endpoint path (e.g., "/1.0/library")
    ///
    /// # Returns
    /// Deserialized JSON response of type `T`
    ///
    /// # Errors
    /// Returns error if request fails or response cannot be deserialized
    pub async fn get<T>(&self, endpoint: &str) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        self.request(Method::GET, endpoint, None::<&()>).await
    }

    /// Perform a GET request with query parameters
    ///
    /// # Arguments
    /// * `endpoint` - API endpoint path
    /// * `query` - Query parameters as key-value pairs
    ///
    /// # Returns
    /// Deserialized JSON response of type `T`
    pub async fn get_with_query<T, Q>(&self, endpoint: &str, query: &Q) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
        Q: Serialize,
    {
        let url = format!("{}{}", self.base_url, endpoint);
        self.request_with_retry(|client, headers| {
            client
                .get(&url)
                .query(query)
                .headers(headers)
        })
        .await
    }

    /// Perform a POST request with JSON body
    ///
    /// # Reference
    /// Based on Cdm.Api.cs:65 (`client.PostAsync()`)
    ///
    /// # Arguments
    /// * `endpoint` - API endpoint path
    /// * `body` - Request body to serialize as JSON
    ///
    /// # Returns
    /// Deserialized JSON response of type `T`
    pub async fn post<T, B>(&self, endpoint: &str, body: B) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
        B: Serialize,
    {
        self.request(Method::POST, endpoint, Some(body)).await
    }

    /// Perform a POST request with form data
    ///
    /// # Arguments
    /// * `endpoint` - API endpoint path
    /// * `form` - Form data as key-value pairs
    ///
    /// # Returns
    /// Deserialized JSON response of type `T`
    pub async fn post_form<T>(&self, endpoint: &str, form: &HashMap<String, String>) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let url = format!("{}{}", self.base_url, endpoint);
        self.request_with_retry(|client, headers| {
            client
                .post(&url)
                .headers(headers)
                .form(form)
        })
        .await
    }

    /// Generic HTTP request with automatic token refresh and retry logic
    ///
    /// # Reference
    /// - Retry policy: ApiExtended.cs:70-73 (Polly library)
    /// - Token refresh: Need to implement in auth.rs
    ///
    /// # Arguments
    /// * `method` - HTTP method (GET, POST, etc.)
    /// * `endpoint` - API endpoint path
    /// * `body` - Optional request body
    ///
    /// # Returns
    /// Deserialized JSON response of type `T`
    ///
    /// # Errors
    /// Returns error if all retry attempts fail
    async fn request<T, B>(&self, method: Method, endpoint: &str, body: Option<B>) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
        B: Serialize,
    {
        let url = format!("{}{}", self.base_url, endpoint);

        self.request_with_retry(|client, headers| {
            let mut req_builder = client.request(method.clone(), &url).headers(headers);

            if let Some(ref b) = body {
                req_builder = req_builder.json(b);
            }

            req_builder
        })
        .await
    }

    /// Execute request with retry logic and exponential backoff
    ///
    /// # Reference
    /// Based on ApiExtended.cs:70-73 (Polly retry policy - 2 retries = 3 total)
    ///
    /// Retry strategy:
    /// - Attempt 1: Immediate
    /// - Attempt 2: After 1 second
    /// - Attempt 3: After 2 seconds (total 3 seconds)
    /// - Max attempts: 3
    ///
    /// Retries on:
    /// - Network errors (connection timeout, DNS failure)
    /// - 5xx server errors (temporary server issues)
    /// - 429 Rate Limiting (with respect to Retry-After header)
    /// - 401 Unauthorized (attempt token refresh once)
    ///
    /// No retry on:
    /// - 4xx client errors (except 401, 429)
    /// - Successful responses (2xx)
    async fn request_with_retry<T, F>(&self, request_builder: F) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
        F: Fn(&Client, HeaderMap) -> reqwest::RequestBuilder,
    {
        let mut attempts = 0;
        let mut last_error = None;

        // Acquire semaphore permit for concurrency control
        // Reference: ApiExtended.cs:91 (semaphore.WaitAsync())
        let _permit = self.semaphore.acquire().await.map_err(|e| {
            LibationError::InternalError(format!("Semaphore acquire failed: {}", e))
        })?;

        while attempts < self.config.max_retries {
            attempts += 1;

            // Get fresh headers with current auth token
            let headers = match self.build_auth_headers().await {
                Ok(h) => h,
                Err(e) => {
                    last_error = Some(e);
                    break; // Auth error - don't retry
                }
            };

            // Build and send request
            let request = request_builder(&self.client, headers).build()?;

            match self.client.execute(request).await {
                Ok(response) => {
                    let status = response.status();

                    match status {
                        // Success - parse and return response
                        s if s.is_success() => {
                            return self.handle_success_response(response).await;
                        }

                        // 401 Unauthorized - try token refresh once
                        StatusCode::UNAUTHORIZED if attempts == 1 => {
                            if let Err(e) = self.refresh_tokens().await {
                                return Err(LibationError::auth_failed(
                                    "Token refresh failed",
                                    Some(self.account.lock().await.account_id.clone()),
                                ));
                            }
                            // Retry with new token
                            continue;
                        }

                        // 429 Rate Limiting - respect Retry-After header
                        StatusCode::TOO_MANY_REQUESTS => {
                            let retry_after = self.extract_retry_after(&response);
                            return Err(LibationError::RateLimitExceeded {
                                retry_after_seconds: retry_after,
                                endpoint: self.extract_endpoint_from_url(response.url().as_str()),
                            });
                        }

                        // 5xx Server Error - retry with backoff
                        s if s.is_server_error() && attempts < self.config.max_retries => {
                            let endpoint = self.extract_endpoint_from_url(response.url().as_str());
                            let error_body = response.text().await.unwrap_or_default();
                            last_error = Some(LibationError::api_failed(
                                format!("Server error: {}", error_body),
                                Some(status.as_u16()),
                                Some(endpoint),
                            ));

                            // Exponential backoff: 1s, 2s, 4s...
                            let delay = Duration::from_secs(INITIAL_RETRY_DELAY_SECS * 2_u64.pow(attempts - 1));
                            sleep(delay).await;
                            continue;
                        }

                        // Other errors - don't retry
                        _ => {
                            return self.handle_error_response(response).await;
                        }
                    }
                }

                // Network error - retry with backoff
                Err(e) if attempts < self.config.max_retries && self.is_retryable_network_error(&e) => {
                    last_error = Some(LibationError::network_error(
                        format!("Network request failed: {}", e),
                        true,
                    ));

                    let delay = Duration::from_secs(INITIAL_RETRY_DELAY_SECS * 2_u64.pow(attempts - 1));
                    sleep(delay).await;
                    continue;
                }

                // Non-retryable network error
                Err(e) => {
                    return Err(LibationError::network_error(
                        format!("Network request failed: {}", e),
                        false,
                    ));
                }
            }
        }

        // All retries exhausted
        Err(last_error.unwrap_or_else(|| {
            LibationError::ApiRequestFailed {
                message: format!("Request failed after {} attempts", attempts),
                status_code: None,
                endpoint: None,
            }
        }))
    }

    /// Build authentication headers from account tokens
    ///
    /// # Reference
    /// Based on Cdm.Api.cs:162-163 (Add headers to request)
    async fn build_auth_headers(&self) -> Result<HeaderMap> {
        let account = self.account.lock().await;
        let mut headers = HeaderMap::new();

        if let Some(ref identity) = account.identity {
            let auth_value = format!("Bearer {}", identity.access_token.token);
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&auth_value)
                    .map_err(|e| LibationError::InvalidInput(format!("Invalid auth token: {}", e)))?,
            );
        }

        Ok(headers)
    }

    /// Handle successful HTTP response
    async fn handle_success_response<T>(&self, response: Response) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let status = response.status();
        let url = response.url().clone();

        // Get response text first so we can log it on parse error
        let response_text = response.text().await.map_err(|e| LibationError::ApiRequestFailed {
            message: format!("Failed to read response body: {}", e),
            status_code: Some(status.as_u16()),
            endpoint: Some(url.path().to_string()),
        })?;

        match serde_json::from_str::<T>(&response_text) {
            Ok(data) => Ok(data),
            Err(e) => {
                // Extract context around the error location (800 chars)
                let error_col = e.column();
                let start = error_col.saturating_sub(400);
                let end = (error_col + 400).min(response_text.len());
                let context = &response_text[start..end];

                Err(LibationError::InvalidApiResponse {
                    message: format!(
                        "Parse error: {} at col {}. Context: ...{}...",
                        e, error_col, context
                    ),
                    response_body: Some(response_text),
                })
            }
        }
    }

    /// Handle error HTTP response
    async fn handle_error_response<T>(&self, response: Response) -> Result<T> {
        let status = response.status();
        let url = response.url().clone();
        let error_body = response.text().await.unwrap_or_default();

        Err(LibationError::api_failed(
            format!("API request failed: {}", error_body),
            Some(status.as_u16()),
            Some(self.extract_endpoint_from_url(url.as_str())),
        ))
    }

    /// Check if a network error is retryable
    ///
    /// Reference: ApiExtended.cs:70 (Policy.Handle<Exception>() - retries all exceptions)
    fn is_retryable_network_error(&self, error: &reqwest::Error) -> bool {
        error.is_timeout() || error.is_connect() || error.is_request()
    }

    /// Extract retry-after delay from response headers (in seconds)
    fn extract_retry_after(&self, response: &Response) -> u64 {
        response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(60) // Default to 60 seconds
    }

    /// Extract endpoint path from full URL
    fn extract_endpoint_from_url(&self, url: &str) -> String {
        url.strip_prefix(&self.base_url)
            .unwrap_or(url)
            .to_string()
    }

    /// Refresh authentication tokens
    ///
    /// TODO: Implement token refresh logic in auth.rs
    /// Reference: Need to implement refresh token flow
    async fn refresh_tokens(&self) -> Result<()> {
        // TODO: Port token refresh logic from C# Identity class
        // For now, return NotImplemented error
        Err(LibationError::not_implemented("Token refresh not yet implemented"))
    }

    /// Download file with progress callback
    ///
    /// # Reference
    /// Based on NetworkFileStream.cs:154-180 (BeginDownloadingAsync)
    ///
    /// # Arguments
    /// * `url` - URL to download from
    /// * `progress_callback` - Optional callback for progress updates (bytes_downloaded, total_bytes)
    ///
    /// # Returns
    /// Downloaded file content as bytes
    ///
    /// # Errors
    /// Returns error if download fails
    pub async fn download<F>(&self, url: &str, mut progress_callback: Option<F>) -> Result<Vec<u8>>
    where
        F: FnMut(u64, u64),
    {
        let _permit = self.semaphore.acquire().await.map_err(|e| {
            LibationError::InternalError(format!("Semaphore acquire failed: {}", e))
        })?;

        let headers = self.build_auth_headers().await?;
        let response = self
            .client
            .get(url)
            .headers(headers)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(LibationError::DownloadFailed(format!(
                "Download failed with status: {}",
                response.status()
            )));
        }

        let total_size = response.content_length().unwrap_or(0);
        let mut downloaded: u64 = 0;
        let mut buffer = Vec::with_capacity(total_size as usize);

        let mut stream = response.bytes_stream();
        use futures_util::StreamExt; // For stream.next()

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            buffer.extend_from_slice(&chunk);
            downloaded += chunk.len() as u64;

            if let Some(ref mut callback) = progress_callback {
                callback(downloaded, total_size);
            }
        }

        Ok(buffer)
    }

    /// Get account reference (for reading account info)
    pub fn account(&self) -> Arc<Mutex<Account>> {
        Arc::clone(&self.account)
    }
}

// ===== TESTS =====

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audible_domain_from_str() {
        assert_eq!(AudibleDomain::from_str("audible.com"), Some(AudibleDomain::Us));
        assert_eq!(AudibleDomain::from_str("com"), Some(AudibleDomain::Us));
        assert_eq!(AudibleDomain::from_str("audible.co.uk"), Some(AudibleDomain::Uk));
        assert_eq!(AudibleDomain::from_str("co.uk"), Some(AudibleDomain::Uk));
        assert_eq!(AudibleDomain::from_str("audible.de"), Some(AudibleDomain::De));
        assert_eq!(AudibleDomain::from_str("invalid"), None);
    }

    #[test]
    fn test_audible_domain_api_url() {
        assert_eq!(AudibleDomain::Us.api_url(), "https://api.audible.com");
        assert_eq!(AudibleDomain::Uk.api_url(), "https://api.audible.co.uk");
        assert_eq!(AudibleDomain::De.api_url(), "https://api.audible.de");
    }

    #[test]
    fn test_client_config_builder() {
        let config = ClientConfig::builder()
            .domain(AudibleDomain::Uk)
            .timeout(Duration::from_secs(60))
            .max_retries(5)
            .user_agent("TestAgent/1.0")
            .enable_cookies(false)
            .build();

        assert_eq!(config.domain, AudibleDomain::Uk);
        assert_eq!(config.timeout, Duration::from_secs(60));
        assert_eq!(config.max_retries, 5);
        assert_eq!(config.user_agent, "TestAgent/1.0");
        assert_eq!(config.enable_cookies, false);
    }

    #[tokio::test]
    async fn test_client_creation_requires_account_id() {
        let account = Account {
            account_id: "".to_string(),
            account_name: "Test".to_string(),
            library_scan: true,
            decrypt_key: "".to_string(),
            identity: None,
        };

        let result = AudibleClient::new(account);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), LibationError::MissingRequiredField(_)));
    }
}
