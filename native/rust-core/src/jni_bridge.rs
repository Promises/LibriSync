// LibriSync - Audible Library Sync for Mobile
// Copyright (C) 2025 Henning Berge
//
// This program is a Rust port of Libation (https://github.com/rmcrackan/Libation)
// Original work Copyright (C) Libation contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.


//! JNI bridge for Android - Exposes Rust core functionality to React Native
//!
//! This module provides JNI wrapper functions that expose the Rust core
//! functionality to the Kotlin Expo module, which is then accessible from
//! React Native JavaScript code.
//!
//! # Architecture
//! JavaScript (React Native) → Kotlin (ExpoRustBridgeModule) → JNI → Rust
//!
//! # Design Patterns
//! 1. **JSON Communication**: All complex data is serialized to JSON for FFI crossing
//! 2. **Error Handling**: All errors are caught and returned as JSON error responses
//! 3. **Async Runtime**: Tokio runtime is used to execute async Rust functions
//! 4. **No Panics**: All panics are caught to prevent crashes across FFI boundary
//!
//! # Response Format
//! All functions return JSON strings with this structure:
//! ```json
//! {
//!   "success": true,
//!   "data": { ... }
//! }
//! ```
//! Or on error:
//! ```json
//! {
//!   "success": false,
//!   "error": "Error message"
//! }
//! ```

use jni::objects::{JClass, JString};
use jni::sys::jstring;
use jni::JNIEnv;
use serde::{Deserialize, Serialize};
use std::panic::{self, AssertUnwindSafe};

// Lazy static tokio runtime for async operations
lazy_static::lazy_static! {
    static ref RUNTIME: tokio::runtime::Runtime =
        tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Convert JString to Rust String
fn jstring_to_string(env: &mut JNIEnv, jstr: JString) -> crate::Result<String> {
    env.get_string(&jstr)
        .map(|s| s.into())
        .map_err(|e| crate::LibationError::InvalidInput(format!("JNI string conversion failed: {}", e)))
}

/// Convert Rust result to JSON response string
fn result_to_json<T: Serialize>(result: crate::Result<T>) -> String {
    match result {
        Ok(data) => serde_json::json!({
            "success": true,
            "data": data
        }).to_string(),
        Err(e) => serde_json::json!({
            "success": false,
            "error": e.to_string()
        }).to_string(),
    }
}

/// Create success response JSON
fn success_response<T: Serialize>(data: T) -> String {
    serde_json::json!({
        "success": true,
        "data": data
    }).to_string()
}

/// Create error response JSON
fn error_response(error: &str) -> String {
    serde_json::json!({
        "success": false,
        "error": error
    }).to_string()
}

/// Wrap a function call with panic catching
fn catch_panic<F>(f: F) -> String
where
    F: FnOnce() -> String,
{
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(panic_err) => {
            let panic_msg = if let Some(s) = panic_err.downcast_ref::<String>() {
                s.clone()
            } else if let Some(s) = panic_err.downcast_ref::<&str>() {
                s.to_string()
            } else {
                "Unknown panic occurred".to_string()
            };
            error_response(&format!("Rust panic: {}", panic_msg))
        }
    }
}

// ============================================================================
// EXISTING TEST FUNCTION (DO NOT MODIFY)
// ============================================================================

#[no_mangle]
pub extern "C" fn Java_expo_modules_rustbridge_ExpoRustBridgeModule_nativeLogFromRust(
    mut env: JNIEnv,
    _class: JClass,
    message: JString,
) -> jstring {
    let input: String = env
        .get_string(&message)
        .expect("Couldn't get java string!")
        .into();

    let result = crate::log_from_rust(input);

    let output = env
        .new_string(result)
        .expect("Couldn't create java string!");

    output.into_raw()
}

// ============================================================================
// AUTHENTICATION FUNCTIONS
// ============================================================================

/// Generate OAuth authorization URL with PKCE
///
/// # Arguments (JSON string)
/// ```json
/// {
///   "locale_code": "us",
///   "device_serial": "1234-5678-9012"
/// }
/// ```
///
/// # Returns (JSON)
/// ```json
/// {
///   "success": true,
///   "data": {
///     "authorization_url": "https://...",
///     "pkce_verifier": "...",
///     "state": "..."
///   }
/// }
/// ```
#[no_mangle]
pub extern "C" fn Java_expo_modules_rustbridge_ExpoRustBridgeModule_nativeGenerateOAuthUrl(
    mut env: JNIEnv,
    _class: JClass,
    params_json: JString,
) -> jstring {
    // Convert JString to String before entering closures (to avoid borrow issues)
    let params_str = match jstring_to_string(&mut env, params_json) {
        Ok(s) => s,
        Err(e) => {
            return env.new_string(error_response(&e.to_string()))
                .expect("Failed to create Java string")
                .into_raw();
        }
    };

    let response = catch_panic(move || {
        #[derive(Deserialize)]
        struct Params {
            locale_code: String,
            device_serial: String,
        }

        match (move || -> crate::Result<String> {
            let params: Params = serde_json::from_str(&params_str)
                .map_err(|e| crate::LibationError::InvalidInput(format!("Invalid JSON: {}", e)))?;

            // Get locale
            let locale = crate::api::auth::Locale::from_country_code(&params.locale_code)
                .ok_or_else(|| crate::LibationError::InvalidInput(format!("Invalid locale: {}", params.locale_code)))?;

            // Generate PKCE and state
            let pkce = crate::api::auth::PkceChallenge::generate()?;
            let state = crate::api::auth::OAuthState::generate();

            // Generate authorization URL
            let auth_url = crate::api::auth::generate_authorization_url(
                &locale,
                &params.device_serial,
                &pkce,
                &state,
            )?;

            let response = serde_json::json!({
                "authorization_url": auth_url,
                "pkce_verifier": pkce.verifier,
                "state": state.value,
            });

            Ok(success_response(response))
        })() {
            Ok(result) => result,
            Err(e) => error_response(&e.to_string()),
        }
    });

    env.new_string(response)
        .expect("Failed to create Java string")
        .into_raw()
}

/// Parse OAuth callback URL to extract authorization code
///
/// # Arguments (JSON string)
/// ```json
/// {
///   "callback_url": "https://localhost/callback?code=..."
/// }
/// ```
///
/// # Returns (JSON)
/// ```json
/// {
///   "success": true,
///   "data": {
///     "authorization_code": "ABC123..."
///   }
/// }
/// ```
#[no_mangle]
pub extern "C" fn Java_expo_modules_rustbridge_ExpoRustBridgeModule_nativeParseOAuthCallback(
    mut env: JNIEnv,
    _class: JClass,
    params_json: JString,
) -> jstring {
    let params_str_result = jstring_to_string(&mut env, params_json);

    let response = catch_panic(move || {
        #[derive(Deserialize)]
        struct Params {
            callback_url: String,
        }

        match (move || -> crate::Result<String> {
            let params_str = params_str_result?;
            let params: Params = serde_json::from_str(&params_str)
                .map_err(|e| crate::LibationError::InvalidInput(format!("Invalid JSON: {}", e)))?;

            let auth_code = crate::api::auth::parse_authorization_callback(&params.callback_url)?;

            let response = serde_json::json!({
                "authorization_code": auth_code,
            });

            Ok(success_response(response))
        })() {
            Ok(result) => result,
            Err(e) => error_response(&e.to_string()),
        }
    });

    env.new_string(response)
        .expect("Failed to create Java string")
        .into_raw()
}

/// Exchange authorization code for complete registration response
///
/// # Arguments (JSON string)
/// ```json
/// {
///   "locale_code": "us",
///   "authorization_code": "ABC123...",
///   "device_serial": "1234-5678-9012",
///   "pkce_verifier": "..."
/// }
/// ```
///
/// # Returns (JSON)
/// ```json
/// {
///   "success": true,
///   "data": {
///     "bearer": {
///       "access_token": "...",
///       "refresh_token": "...",
///       "expires_in": "3600"
///     },
///     "mac_dms": {
///       "device_private_key": "...",
///       "adp_token": "..."
///     },
///     "website_cookies": [...],
///     "store_authentication_cookie": { "cookie": "..." },
///     "device_info": { ... },
///     "customer_info": { ... }
///   }
/// }
/// ```
#[no_mangle]
pub extern "C" fn Java_expo_modules_rustbridge_ExpoRustBridgeModule_nativeExchangeAuthCode(
    mut env: JNIEnv,
    _class: JClass,
    params_json: JString,
) -> jstring {
    let params_str_result = jstring_to_string(&mut env, params_json);

    let response = catch_panic(move || {
        #[derive(Deserialize)]
        struct Params {
            locale_code: String,
            authorization_code: String,
            device_serial: String,
            pkce_verifier: String,
        }

        match (move || -> crate::Result<String> {
            let params_str = params_str_result?;
            let params: Params = serde_json::from_str(&params_str)
                .map_err(|e| crate::LibationError::InvalidInput(format!("Invalid JSON: {}", e)))?;

            let locale = crate::api::auth::Locale::from_country_code(&params.locale_code)
                .ok_or_else(|| crate::LibationError::InvalidInput(format!("Invalid locale: {}", params.locale_code)))?;

            let pkce = crate::api::auth::PkceChallenge {
                verifier: params.pkce_verifier,
                challenge: String::new(), // Not needed for exchange
                method: "S256".to_string(),
            };

            let result = RUNTIME.block_on(async {
                crate::api::auth::exchange_authorization_code(
                    &locale,
                    &params.authorization_code,
                    &params.device_serial,
                    &pkce,
                ).await
            })?;

            Ok(success_response(result))
        })() {
            Ok(result) => result,
            Err(e) => error_response(&e.to_string()),
        }
    });

    env.new_string(response)
        .expect("Failed to create Java string")
        .into_raw()
}

/// Refresh access token using refresh token
///
/// # Arguments (JSON string)
/// ```json
/// {
///   "locale_code": "us",
///   "refresh_token": "...",
///   "device_serial": "1234-5678-9012"
/// }
/// ```
///
/// # Returns (JSON)
/// ```json
/// {
///   "success": true,
///   "data": {
///     "access_token": "...",
///     "refresh_token": "...",
///     "expires_in": 3600,
///     "token_type": "Bearer"
///   }
/// }
/// ```
#[no_mangle]
pub extern "C" fn Java_expo_modules_rustbridge_ExpoRustBridgeModule_nativeRefreshAccessToken(
    mut env: JNIEnv,
    _class: JClass,
    params_json: JString,
) -> jstring {
    let params_str_result = jstring_to_string(&mut env, params_json);

    let response = catch_panic(move || {
        #[derive(Deserialize)]
        struct Params {
            locale_code: String,
            refresh_token: String,
            device_serial: String,
        }

        match (move || -> crate::Result<String> {
            let params_str = params_str_result?;
            let params: Params = serde_json::from_str(&params_str)
                .map_err(|e| crate::LibationError::InvalidInput(format!("Invalid JSON: {}", e)))?;

            let locale = crate::api::auth::Locale::from_country_code(&params.locale_code)
                .ok_or_else(|| crate::LibationError::InvalidInput(format!("Invalid locale: {}", params.locale_code)))?;

            let result = RUNTIME.block_on(async {
                crate::api::auth::refresh_access_token(
                    &locale,
                    &params.refresh_token,
                    &params.device_serial,
                ).await
            })?;

            Ok(success_response(result))
        })() {
            Ok(result) => result,
            Err(e) => error_response(&e.to_string()),
        }
    });

    env.new_string(response)
        .expect("Failed to create Java string")
        .into_raw()
}

/// Get activation bytes for DRM decryption
///
/// # Arguments (JSON string)
/// ```json
/// {
///   "locale_code": "us",
///   "access_token": "..."
/// }
/// ```
///
/// # Returns (JSON)
/// ```json
/// {
///   "success": true,
///   "data": {
///     "activation_bytes": "1CEB00DA"
///   }
/// }
/// ```
#[no_mangle]
pub extern "C" fn Java_expo_modules_rustbridge_ExpoRustBridgeModule_nativeGetActivationBytes(
    mut env: JNIEnv,
    _class: JClass,
    params_json: JString,
) -> jstring {
    let params_str_result = jstring_to_string(&mut env, params_json);

    let response = catch_panic(move || {
        #[derive(Deserialize)]
        struct Params {
            locale_code: String,
            access_token: String,
        }

        match (move || -> crate::Result<String> {
            let params_str = params_str_result?;
            let params: Params = serde_json::from_str(&params_str)
                .map_err(|e| crate::LibationError::InvalidInput(format!("Invalid JSON: {}", e)))?;

            let locale = crate::api::auth::Locale::from_country_code(&params.locale_code)
                .ok_or_else(|| crate::LibationError::InvalidInput(format!("Invalid locale: {}", params.locale_code)))?;

            let result = RUNTIME.block_on(async {
                crate::api::auth::get_activation_bytes(&locale, &params.access_token).await
            })?;

            let response = serde_json::json!({
                "activation_bytes": result,
            });

            Ok(success_response(response))
        })() {
            Ok(result) => result,
            Err(e) => error_response(&e.to_string()),
        }
    });

    env.new_string(response)
        .expect("Failed to create Java string")
        .into_raw()
}

// ============================================================================
// LIBRARY FUNCTIONS
// ============================================================================

// Database functions - UnwindSafe issues fixed with AssertUnwindSafe

/// Synchronize library from Audible API
///
/// # Arguments (JSON string)
/// ```json
/// {
///   "db_path": "/data/data/.../libation.db",
///   "account_json": "{...}" // serialized Account object
/// }
/// ```
///
/// # Returns (JSON)
/// ```json
/// {
///   "success": true,
///   "data": {
///     "total_items": 150,
///     "books_added": 10,
///     "books_updated": 140,
///     "books_absent": 0,
///     "errors": []
///   }
/// }
/// ```
#[no_mangle]
pub extern "C" fn Java_expo_modules_rustbridge_ExpoRustBridgeModule_nativeSyncLibrary(
    mut env: JNIEnv,
    _class: JClass,
    params_json: JString,
) -> jstring {
    let params_str_result = jstring_to_string(&mut env, params_json);

    let response = catch_panic(move || {
        #[derive(Deserialize)]
        struct Params {
            db_path: String,
            account_json: String,
        }

        match (move || -> crate::Result<String> {
            let params_str = params_str_result?;
            let params: Params = serde_json::from_str(&params_str)
                .map_err(|e| crate::LibationError::InvalidInput(format!("Invalid JSON: {}", e)))?;

            let account: crate::api::auth::Account = serde_json::from_str(&params.account_json)
                .map_err(|e| crate::LibationError::InvalidInput(format!("Invalid account JSON: {}", e)))?;

            let result = RUNTIME.block_on(async {
                let db = crate::storage::Database::new(&params.db_path).await?;

                let mut client = crate::api::client::AudibleClient::new(account.clone())?;

                client.sync_library(&db, &account).await
            })?;

            Ok(success_response(result))
        })() {
            Ok(result) => result,
            Err(e) => error_response(&e.to_string()),
        }
    });

    env.new_string(response)
        .expect("Failed to create Java string")
        .into_raw()
}

/// Synchronize a single page of library from Audible API
///
/// This allows for progressive UI updates by fetching one page at a time.
///
/// # Arguments (JSON string)
/// ```json
/// {
///   "db_path": "/data/data/.../libation.db",
///   "account_json": "{...}", // serialized Account object
///   "page": 1 // page number (1-indexed)
/// }
/// ```
///
/// # Returns (JSON)
/// ```json
/// {
///   "success": true,
///   "data": {
///     "total_items": 50,
///     "total_library_count": 150,
///     "books_added": 10,
///     "books_updated": 40,
///     "books_absent": 0,
///     "errors": [],
///     "has_more": true
///   }
/// }
/// ```
#[no_mangle]
pub extern "C" fn Java_expo_modules_rustbridge_ExpoRustBridgeModule_nativeSyncLibraryPage(
    mut env: JNIEnv,
    _class: JClass,
    params_json: JString,
) -> jstring {
    let params_str_result = jstring_to_string(&mut env, params_json);

    let response = catch_panic(move || {
        #[derive(Deserialize)]
        struct Params {
            db_path: String,
            account_json: String,
            page: i32,
        }

        match (move || -> crate::Result<String> {
            let params_str = params_str_result?;
            let params: Params = serde_json::from_str(&params_str)
                .map_err(|e| crate::LibationError::InvalidInput(format!("Invalid JSON: {}", e)))?;

            let account: crate::api::auth::Account = serde_json::from_str(&params.account_json)
                .map_err(|e| crate::LibationError::InvalidInput(format!("Invalid account JSON: {}", e)))?;

            let result = RUNTIME.block_on(async {
                let db = crate::storage::Database::new(&params.db_path).await?;

                let mut client = crate::api::client::AudibleClient::new(account.clone())?;

                client.sync_library_page(&db, &account, params.page).await
            })?;

            Ok(success_response(result))
        })() {
            Ok(result) => result,
            Err(e) => error_response(&e.to_string()),
        }
    });

    env.new_string(response)
        .expect("Failed to create Java string")
        .into_raw()
}

/// Get books from database with pagination
///
/// # Arguments (JSON string)
/// ```json
/// {
///   "db_path": "/data/data/.../libation.db",
///   "offset": 0,
///   "limit": 50
/// }
/// ```
///
/// # Returns (JSON)
/// ```json
/// {
///   "success": true,
///   "data": {
///     "books": [...],
///     "total_count": 150
///   }
/// }
/// ```
#[no_mangle]
pub extern "C" fn Java_expo_modules_rustbridge_ExpoRustBridgeModule_nativeGetBooks(
    mut env: JNIEnv,
    _class: JClass,
    params_json: JString,
) -> jstring {
    let params_str_result = jstring_to_string(&mut env, params_json);

    let response = catch_panic(move || {
        #[derive(Deserialize)]
        struct Params {
            db_path: String,
            offset: i64,
            limit: i64,
        }

        match (move || -> crate::Result<String> {
            let params_str = params_str_result?;
            let params: Params = serde_json::from_str(&params_str)
                .map_err(|e| crate::LibationError::InvalidInput(format!("Invalid JSON: {}", e)))?;

            let result = RUNTIME.block_on(async {
                let db = crate::storage::Database::new(&params.db_path).await?;
                let books = crate::storage::queries::list_books_with_relations(db.pool(), params.limit, params.offset).await?;
                let total_count = crate::storage::queries::count_books(db.pool()).await?;

                // Convert BookWithRelations to JSON with arrays for authors/narrators
                let books_json: Vec<serde_json::Value> = books.iter().map(|book| {
                    serde_json::json!({
                        "id": book.book_id,
                        "audible_product_id": book.audible_product_id,
                        "title": book.title,
                        "subtitle": book.subtitle,
                        "description": book.description,
                        "duration_seconds": book.length_in_minutes * 60,
                        "language": book.language,
                        "rating": book.rating_overall,
                        "cover_url": book.picture_large,
                        "release_date": book.date_published,
                        "purchase_date": book.purchase_date,
                        "created_at": book.created_at,
                        "updated_at": book.updated_at,
                        "authors": book.authors_str.as_ref()
                            .map(|s| s.split(", ").filter(|a| !a.is_empty()).collect::<Vec<_>>())
                            .unwrap_or_default(),
                        "narrators": book.narrators_str.as_ref()
                            .map(|s| s.split(", ").filter(|n| !n.is_empty()).collect::<Vec<_>>())
                            .unwrap_or_default(),
                        "publisher": book.publisher,
                        "series_name": book.series_name,
                        "series_sequence": book.series_sequence,
                        "file_path": null,  // TODO: Add when download manager implemented
                        "pdf_url": book.pdf_url,
                        "is_finished": book.is_finished,
                        "is_downloadable": book.is_downloadable,
                        "is_ayce": book.is_ayce,
                        "origin_asin": book.origin_asin,
                        "episode_number": book.episode_number,
                        "content_delivery_type": book.content_delivery_type,
                        "is_abridged": book.is_abridged,
                        "is_spatial": book.is_spatial,
                    })
                }).collect();

                let response = serde_json::json!({
                    "books": books_json,
                    "total_count": total_count,
                });

                Ok::<_, crate::LibationError>(response)
            })?;

            Ok(success_response(result))
        })() {
            Ok(result) => result,
            Err(e) => error_response(&e.to_string()),
        }
    });

    env.new_string(response)
        .expect("Failed to create Java string")
        .into_raw()
}

/// Search books by title
///
/// # Arguments (JSON string)
/// ```json
/// {
///   "db_path": "/data/data/.../libation.db",
///   "query": "harry potter",
///   "limit": 20
/// }
/// ```
///
/// # Returns (JSON)
/// ```json
/// {
///   "success": true,
///   "data": {
///     "books": [...]
///   }
/// }
/// ```
#[no_mangle]
pub extern "C" fn Java_expo_modules_rustbridge_ExpoRustBridgeModule_nativeSearchBooks(
    mut env: JNIEnv,
    _class: JClass,
    params_json: JString,
) -> jstring {
    let params_str_result = jstring_to_string(&mut env, params_json);

    let response = catch_panic(move || {
        #[derive(Deserialize)]
        struct Params {
            db_path: String,
            query: String,
            limit: i64,
        }

        match (move || -> crate::Result<String> {
            let params_str = params_str_result?;
            let params: Params = serde_json::from_str(&params_str)
                .map_err(|e| crate::LibationError::InvalidInput(format!("Invalid JSON: {}", e)))?;

            let result = RUNTIME.block_on(async {
                let db = crate::storage::Database::new(&params.db_path).await?;
                let books = crate::storage::queries::search_books_by_title(
                    db.pool(),
                    &params.query,
                    params.limit,
                ).await?;

                let response = serde_json::json!({
                    "books": books,
                });

                Ok::<_, crate::LibationError>(response)
            })?;

            Ok(success_response(result))
        })() {
            Ok(result) => result,
            Err(e) => error_response(&e.to_string()),
        }
    });

    env.new_string(response)
        .expect("Failed to create Java string")
        .into_raw()
}

// ============================================================================
// DOWNLOAD FUNCTIONS
// ============================================================================

/// Download audiobook file
///
/// # Arguments (JSON string)
/// ```json
/// {
///   "asin": "B012345678",
///   "access_token": "...",
///   "locale_code": "us",
///   "output_path": "/storage/emulated/0/Download/book.aax"
/// }
/// ```
///
/// # Returns (JSON)
/// ```json
/// {
///   "success": true,
///   "data": {
///     "bytes_downloaded": 123456789,
///     "output_path": "/storage/emulated/0/Download/book.aax"
///   }
/// }
/// ```
#[no_mangle]
// ============================================================================
// DECRYPTION FUNCTIONS
// ============================================================================

/// Decrypt AAX file to M4B using activation bytes
///
/// # Arguments (JSON string)
/// ```json
/// {
///   "input_path": "/storage/emulated/0/Download/book.aax",
///   "output_path": "/storage/emulated/0/Download/book.m4b",
///   "activation_bytes": "1CEB00DA"
/// }
/// ```
///
/// # Returns (JSON)
/// ```json
/// {
///   "success": true,
///   "data": {
///     "output_path": "/storage/emulated/0/Download/book.m4b",
///     "file_size": 123456789
///   }
/// }
/// ```
#[no_mangle]
pub extern "C" fn Java_expo_modules_rustbridge_ExpoRustBridgeModule_nativeDecryptAAX(
    mut env: JNIEnv,
    _class: JClass,
    params_json: JString,
) -> jstring {
    let params_str_result = jstring_to_string(&mut env, params_json);

    let response = catch_panic(move || {
        #[derive(Deserialize)]
        struct Params {
            input_path: String,
            output_path: String,
            activation_bytes: String,
        }

        match (move || -> crate::Result<String> {
            let params_str = params_str_result?;
            let params: Params = serde_json::from_str(&params_str)
                .map_err(|e| crate::LibationError::InvalidInput(format!("Invalid JSON: {}", e)))?;

            let activation_bytes = crate::crypto::activation::ActivationBytes::from_hex(&params.activation_bytes)?;

            let result = RUNTIME.block_on(async {
                let decrypter = crate::crypto::aax::AaxDecrypter::new(activation_bytes);

                let input_path = std::path::Path::new(&params.input_path);
                let output_path = std::path::Path::new(&params.output_path);

                decrypter.decrypt_file(input_path, output_path).await?;

                let file_size = tokio::fs::metadata(output_path)
                    .await
                    .map(|m| m.len())
                    .unwrap_or(0);

                let response = serde_json::json!({
                    "output_path": params.output_path,
                    "file_size": file_size,
                });

                Ok::<_, crate::LibationError>(response)
            })?;

            Ok(success_response(result))
        })() {
            Ok(result) => result,
            Err(e) => error_response(&e.to_string()),
        }
    });

    env.new_string(response)
        .expect("Failed to create Java string")
        .into_raw()
}

// ============================================================================
// DATABASE FUNCTIONS
// ============================================================================

/// Initialize database at specified path
///
/// # Arguments (JSON string)
/// ```json
/// {
///   "db_path": "/data/data/.../libation.db"
/// }
/// ```
///
/// # Returns (JSON)
/// ```json
/// {
///   "success": true,
///   "data": {
///     "initialized": true
///   }
/// }
/// ```
#[no_mangle]
pub extern "C" fn Java_expo_modules_rustbridge_ExpoRustBridgeModule_nativeInitDatabase(
    mut env: JNIEnv,
    _class: JClass,
    params_json: JString,
) -> jstring {
    let params_str_result = jstring_to_string(&mut env, params_json);

    let response = catch_panic(move || {
        #[derive(Deserialize)]
        struct Params {
            db_path: String,
        }

        match (move || -> crate::Result<String> {
            let params_str = params_str_result?;
            let params: Params = serde_json::from_str(&params_str)
                .map_err(|e| crate::LibationError::InvalidInput(format!("Invalid JSON: {}", e)))?;

            let result = RUNTIME.block_on(async {
                let _db = crate::storage::Database::new(&params.db_path).await?;

                let response = serde_json::json!({
                    "initialized": true,
                });

                Ok::<_, crate::LibationError>(response)
            })?;

            Ok(success_response(result))
        })() {
            Ok(result) => result,
            Err(e) => error_response(&e.to_string()),
        }
    });

    env.new_string(response)
        .expect("Failed to create Java string")
        .into_raw()
}

// ============================================================================
// UTILITY FUNCTIONS
// ============================================================================

/// Validate activation bytes format
///
/// # Arguments (JSON string)
/// ```json
/// {
///   "activation_bytes": "1CEB00DA"
/// }
/// ```
///
/// # Returns (JSON)
/// ```json
/// {
///   "success": true,
///   "data": {
///     "valid": true
///   }
/// }
/// ```
#[no_mangle]
pub extern "C" fn Java_expo_modules_rustbridge_ExpoRustBridgeModule_nativeValidateActivationBytes(
    mut env: JNIEnv,
    _class: JClass,
    params_json: JString,
) -> jstring {
    let params_str_result = jstring_to_string(&mut env, params_json);

    let response = catch_panic(move || {
        #[derive(Deserialize)]
        struct Params {
            activation_bytes: String,
        }

        match (move || -> crate::Result<String> {
            let params_str = params_str_result?;
            let params: Params = serde_json::from_str(&params_str)
                .map_err(|e| crate::LibationError::InvalidInput(format!("Invalid JSON: {}", e)))?;

            let valid = crate::crypto::activation::ActivationBytes::from_hex(&params.activation_bytes).is_ok();

            let response = serde_json::json!({
                "valid": valid,
            });

            Ok(success_response(response))
        })() {
            Ok(result) => result,
            Err(e) => error_response(&e.to_string()),
        }
    });

    env.new_string(response)
        .expect("Failed to create Java string")
        .into_raw()
}

/// Get list of supported locales
///
/// # Arguments (JSON string)
/// ```json
/// {}
/// ```
///
/// # Returns (JSON)
/// ```json
/// {
///   "success": true,
///   "data": {
///     "locales": [
///       {"country_code": "us", "name": "United States", "domain": "audible.com"},
///       ...
///     ]
///   }
/// }
/// ```
#[no_mangle]
pub extern "C" fn Java_expo_modules_rustbridge_ExpoRustBridgeModule_nativeGetSupportedLocales(
    mut env: JNIEnv,
    _class: JClass,
    _params_json: JString,
) -> jstring {
    let response = catch_panic(move || {
        let locales = crate::api::auth::Locale::all();

        let response = serde_json::json!({
            "locales": locales,
        });

        success_response(response)
    });

    env.new_string(response)
        .expect("Failed to create Java string")
        .into_raw()
}

/// Get customer information from Audible API
///
/// # Arguments (JSON string)
/// ```json
/// {
///   "locale_code": "us",
///   "access_token": "Atna|..."
/// }
/// ```
///
/// # Returns (JSON)
/// ```json
/// {
///   "success": true,
///   "data": {
///     "name": "John Doe",
///     "given_name": "John",
///     "email": "john@example.com"
///   }
/// }
/// ```
#[no_mangle]
pub extern "C" fn Java_expo_modules_rustbridge_ExpoRustBridgeModule_nativeGetCustomerInformation(
    mut env: JNIEnv,
    _class: JClass,
    params_json: JString,
) -> jstring {
    let params_str_result = jstring_to_string(&mut env, params_json);

    let response = catch_panic(move || {
        #[derive(Deserialize)]
        struct Params {
            locale_code: String,
            access_token: String,
        }

        match (move || -> crate::Result<String> {
            let params_str = params_str_result?;
            let params: Params = serde_json::from_str(&params_str)
                .map_err(|e| crate::LibationError::InvalidInput(format!("Invalid JSON: {}", e)))?;

            let result = RUNTIME.block_on(async {
                // Get locale
                let locale = match params.locale_code.as_str() {
                    "us" => crate::api::auth::Locale::us(),
                    "uk" => crate::api::auth::Locale::uk(),
                    "de" => crate::api::auth::Locale::de(),
                    "fr" => crate::api::auth::Locale::fr(),
                    "ca" => crate::api::auth::Locale::ca(),
                    "au" => crate::api::auth::Locale::au(),
                    "it" => crate::api::auth::Locale::it(),
                    "es" => crate::api::auth::Locale::es(),
                    "in" => crate::api::auth::Locale::in_(),
                    "jp" => crate::api::auth::Locale::jp(),
                    _ => return Err(crate::LibationError::InvalidInput(format!("Unknown locale: {}", params.locale_code))),
                };

                // Create identity with access token
                let access_token = crate::api::auth::AccessToken {
                    token: params.access_token.clone(),
                    expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
                };

                let identity = crate::api::auth::Identity::new(
                    access_token,
                    String::new(), // refresh_token - not needed for this call
                    String::new(), // device_private_key - not needed
                    String::new(), // adp_token - not needed
                    locale.clone(),
                );

                // Create account with identity
                let account = crate::api::auth::Account {
                    account_id: "temp".to_string(),
                    account_name: "temp".to_string(),
                    library_scan: true,
                    decrypt_key: String::new(),
                    identity: Some(identity),
                };

                let client = crate::api::client::AudibleClient::new(account)?;
                let customer_info = client.get_customer_information().await?;

                Ok::<_, crate::LibationError>(customer_info)
            })?;

            Ok(success_response(result))
        })() {
            Ok(result) => result,
            Err(e) => error_response(&e.to_string()),
        }
    });

    env.new_string(response)
        .expect("Failed to create Java string")
        .into_raw()
}

/// Download and decrypt an audiobook
///
/// # Parameters
/// JSON string with:
/// ```json
/// {
///   "accountJson": "{ ... }",  // Complete account JSON with identity
///   "asin": "B07T2F8VJM",
///   "outputDirectory": "/path/to/save",
///   "quality": "High"  // "Low", "Normal", "High", "Extreme"
/// }
/// ```
///
/// # Returns
/// JSON response:
/// ```json
/// {
///   "success": true,
///   "data": {
///     "outputPath": "/path/to/file.m4b",
///     "fileSize": 148080000,
///     "duration": 9783.3
///   }
/// }
/// ```
#[no_mangle]
pub extern "C" fn Java_expo_modules_rustbridge_ExpoRustBridgeModule_nativeDownloadBook(
    mut env: JNIEnv,
    _class: JClass,
    params_json: JString,
) -> jstring {
    let params_str_result = jstring_to_string(&mut env, params_json);

    let response = catch_panic(move || {
        #[derive(Deserialize)]
        struct Params {
            #[serde(rename = "accountJson")]
            account_json: String,
            asin: String,
            #[serde(rename = "outputDirectory")]
            output_directory: String,
            quality: String,
        }

        match (move || -> crate::Result<String> {
            let params_str = params_str_result?;
            let params: Params = serde_json::from_str(&params_str)
                .map_err(|e| crate::LibationError::InvalidInput(format!("Invalid JSON: {}", e)))?;

            let result = RUNTIME.block_on(async {
                // Parse account
                let account: crate::api::auth::Account = serde_json::from_str(&params.account_json)
                    .map_err(|e| crate::LibationError::InvalidInput(format!("Invalid account JSON: {}", e)))?;

                // Parse quality
                let quality = match params.quality.as_str() {
                    "Low" => crate::api::content::DownloadQuality::Low,
                    "Normal" => crate::api::content::DownloadQuality::Normal,
                    "High" => crate::api::content::DownloadQuality::High,
                    "Extreme" => crate::api::content::DownloadQuality::Extreme,
                    _ => crate::api::content::DownloadQuality::High,
                };

                // Create client
                let client = crate::api::client::AudibleClient::new(account)?;

                // Get download license
                let license = client.build_download_license(&params.asin, quality, false).await?;

                // Extract AAXC keys
                let (key_hex, iv_hex) = if let Some(ref keys) = license.decryption_keys {
                    if !keys.is_empty() && keys[0].key_part_1.len() == 16 {
                        let key = keys[0].key_part_1.iter()
                            .map(|b| format!("{:02x}", b))
                            .collect::<String>();
                        let iv = if let Some(ref iv_bytes) = keys[0].key_part_2 {
                            iv_bytes.iter()
                                .map(|b| format!("{:02x}", b))
                                .collect::<String>()
                        } else {
                            return Err(crate::LibationError::InvalidInput("No IV in AAXC keys".to_string()));
                        };
                        (key, iv)
                    } else {
                        return Err(crate::LibationError::InvalidInput("Unsupported key format (only AAXC supported)".to_string()));
                    }
                } else {
                    return Err(crate::LibationError::InvalidInput("No decryption keys in license".to_string()));
                };

                // Download encrypted file to cache directory
                // (TypeScript layer will copy to user's chosen directory after decryption)
                let cache_dir = std::env::var("TMPDIR")
                    .or_else(|_| std::env::var("TEMP"))
                    .unwrap_or_else(|_| "/data/local/tmp".to_string());

                // Create audiobooks subdirectory in cache
                let audiobooks_cache = format!("{}/audiobooks", cache_dir.trim_end_matches('/'));
                let _ = std::fs::create_dir_all(&audiobooks_cache);

                let encrypted_path = format!("{}/{}.aax", audiobooks_cache, params.asin);
                let decrypted_path = format!("{}/{}.m4b", audiobooks_cache, params.asin);

                // Download with reqwest
                let user_agent = "Audible/671 CFNetwork/1240.0.4 Darwin/20.6.0";
                let http_client = reqwest::Client::new();
                let response = http_client
                    .get(&license.download_url)
                    .header("User-Agent", user_agent)
                    .send()
                    .await
                    .map_err(|e| crate::LibationError::NetworkError {
                        message: format!("Download request failed: {}", e),
                        is_transient: true,
                    })?;

                if !response.status().is_success() {
                    return Err(crate::LibationError::NetworkError {
                        message: format!("HTTP {}", response.status()),
                        is_transient: false,
                    });
                }

                use futures_util::StreamExt;
                use tokio::io::AsyncWriteExt;

                let mut file = tokio::fs::File::create(&encrypted_path).await
                    .map_err(|e| crate::LibationError::internal(format!("Failed to create file {}: {}", encrypted_path, e)))?;

                let mut stream = response.bytes_stream();
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk
                        .map_err(|e| crate::LibationError::NetworkError {
                            message: format!("Stream error: {}", e),
                            is_transient: true,
                        })?;
                    file.write_all(&chunk).await
                        .map_err(|e| crate::LibationError::internal(format!("Write failed: {}", e)))?;
                }
                file.flush().await
                    .map_err(|e| crate::LibationError::internal(format!("Flush failed: {}", e)))?;

                // Return encrypted file path and decryption keys
                // The TypeScript/Kotlin layer will use FFmpeg-Kit to decrypt
                let metadata = tokio::fs::metadata(&encrypted_path).await
                    .map_err(|e| crate::LibationError::not_found(format!("Downloaded file not found: {}", encrypted_path)))?;

                // Return decryption info for FFmpeg-Kit to use
                #[derive(Serialize)]
                struct DownloadResultWithKeys {
                    #[serde(rename = "encryptedPath")]
                    encrypted_path: String,
                    #[serde(rename = "outputPath")]
                    output_path: String,
                    #[serde(rename = "fileSize")]
                    file_size: u64,
                    #[serde(rename = "aaxcKey")]
                    aaxc_key: String,
                    #[serde(rename = "aaxcIv")]
                    aaxc_iv: String,
                }

                Ok::<_, crate::LibationError>(DownloadResultWithKeys {
                    encrypted_path,
                    output_path: decrypted_path,
                    file_size: metadata.len(),
                    aaxc_key: key_hex,
                    aaxc_iv: iv_hex,
                })
            })?;

            Ok(success_response(result))
        })() {
            Ok(result) => result,
            Err(e) => error_response(&e.to_string()),
        }
    });

    env.new_string(response)
        .expect("Failed to create Java string")
        .into_raw()
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_success_response() {
        let response = success_response(serde_json::json!({"test": "data"}));
        assert!(response.contains("\"success\":true"));
        assert!(response.contains("\"test\":\"data\""));
    }

    #[test]
    fn test_error_response() {
        let response = error_response("Test error");
        assert!(response.contains("\"success\":false"));
        assert!(response.contains("Test error"));
    }

    #[test]
    fn test_result_to_json_success() {
        let result: crate::Result<String> = Ok("test".to_string());
        let json = result_to_json(result);
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("test"));
    }

    #[test]
    fn test_result_to_json_error() {
        let result: crate::Result<String> = Err(crate::LibationError::InvalidInput("test error".to_string()));
        let json = result_to_json(result);
        assert!(json.contains("\"success\":false"));
        assert!(json.contains("test error"));
    }

    #[test]
    fn test_catch_panic_normal() {
        let result = catch_panic(|| "normal result".to_string());
        assert_eq!(result, "normal result");
    }

    #[test]
    fn test_catch_panic_with_panic() {
        let result = catch_panic(|| {
            panic!("test panic");
        });
        assert!(result.contains("\"success\":false"));
        assert!(result.contains("test panic"));
    }
}
