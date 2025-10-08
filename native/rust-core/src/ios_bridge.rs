//! C FFI bridge for iOS - Exposes Rust core functionality to React Native
//!
//! This module provides C-compatible FFI wrapper functions that expose the Rust core
//! functionality to Swift/Objective-C code in the Expo module, which is then accessible
//! from React Native JavaScript code.
//!
//! # Architecture
//! JavaScript (React Native) → Swift (ExpoRustBridgeModule) → C FFI → Rust
//!
//! # Design Patterns
//! 1. **JSON Communication**: All complex data is serialized to JSON for FFI crossing
//! 2. **Error Handling**: All errors are caught and returned as JSON error responses
//! 3. **Async Runtime**: Tokio runtime is used to execute async Rust functions
//! 4. **No Panics**: All panics are caught to prevent crashes across FFI boundary
//! 5. **Memory Safety**: All returned strings must be freed by caller using `rust_free_string()`
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
//!
//! # Memory Management
//! **CRITICAL**: All string pointers returned from Rust functions MUST be freed
//! by the caller using `rust_free_string()`. Failure to do so will cause memory leaks.
//!
//! Example Swift code:
//! ```swift
//! let resultPtr = rust_generate_oauth_url(locale, serial)
//! defer { rust_free_string(resultPtr) }
//! let jsonString = String(cString: resultPtr)
//! ```

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic;
use std::collections::HashMap;
use serde::{Deserialize, Serialize};

// Lazy static tokio runtime for async operations
lazy_static::lazy_static! {
    static ref RUNTIME: tokio::runtime::Runtime =
        tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Safely convert C string pointer to Rust String
///
/// # Safety
/// Caller must ensure ptr is a valid null-terminated C string
fn c_str_to_string(ptr: *const c_char) -> crate::Result<String> {
    if ptr.is_null() {
        return Err(crate::LibationError::InvalidInput("Null pointer received".to_string()));
    }
    unsafe {
        CStr::from_ptr(ptr)
            .to_str()
            .map(|s| s.to_string())
            .map_err(|e| crate::LibationError::InvalidInput(format!("Invalid UTF-8: {}", e)))
    }
}

/// Convert Rust string to C string pointer
///
/// # Safety
/// Caller MUST free the returned pointer using `rust_free_string()`
fn string_to_c_str(s: String) -> *mut c_char {
    match CString::new(s) {
        Ok(c_string) => c_string.into_raw(),
        Err(_) => {
            // If string contains null bytes, return error JSON
            let error_json = error_response("String contains null bytes");
            CString::new(error_json).unwrap().into_raw()
        }
    }
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
    F: FnOnce() -> crate::Result<String> + panic::UnwindSafe,
{
    match panic::catch_unwind(f) {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => error_response(&e.to_string()),
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
// AUTHENTICATION FUNCTIONS
// ============================================================================

/// Generate OAuth authorization URL for Audible login
///
/// # Arguments
/// * `locale_code` - Audible locale (e.g., "us", "uk", "de")
/// * `device_serial` - 32-character hex device serial
///
/// # Returns
/// JSON string with format:
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
///
/// # Safety
/// Caller must free the returned string with `rust_free_string()`
#[no_mangle]
pub extern "C" fn rust_generate_oauth_url(
    locale_code: *const c_char,
    device_serial: *const c_char,
) -> *mut c_char {
    let response = catch_panic(|| {
        let locale_code = c_str_to_string(locale_code)?;
        let device_serial = c_str_to_string(device_serial)?;

        // Get locale
        let locale = crate::api::auth::Locale::from_country_code(&locale_code)
            .ok_or_else(|| crate::LibationError::InvalidInput(format!("Invalid locale: {}", locale_code)))?;

        // Generate PKCE and state
        let pkce = crate::api::auth::PkceChallenge::generate()?;
        let state = crate::api::auth::OAuthState::generate();

        // Generate authorization URL
        let auth_url = crate::api::auth::generate_authorization_url(
            &locale,
            &device_serial,
            &pkce,
            &state,
        )?;

        let response = serde_json::json!({
            "authorization_url": auth_url,
            "pkce_verifier": pkce.verifier,
            "state": state.value,
        });

        Ok(success_response(response))
    });

    string_to_c_str(response)
}

/// Parse OAuth callback URL to extract authorization code
///
/// # Arguments
/// * `callback_url` - Full callback URL with authorization code
///
/// # Returns
/// JSON string with format:
/// ```json
/// {
///   "success": true,
///   "data": {
///     "authorization_code": "ABC123..."
///   }
/// }
/// ```
///
/// # Safety
/// Caller must free the returned string with `rust_free_string()`
#[no_mangle]
pub extern "C" fn rust_parse_oauth_callback(callback_url: *const c_char) -> *mut c_char {
    let response = catch_panic(|| {
        let callback_url = c_str_to_string(callback_url)?;

        let auth_code = crate::api::auth::parse_authorization_callback(&callback_url)?;

        let response = serde_json::json!({
            "authorization_code": auth_code,
        });

        Ok(success_response(response))
    });

    string_to_c_str(response)
}

/// Exchange authorization code for complete registration response
///
/// # Arguments
/// * `locale_code` - Audible locale (e.g., "us", "uk", "de")
/// * `auth_code` - Authorization code from OAuth callback
/// * `device_serial` - 32-character hex device serial
/// * `pkce_verifier` - PKCE verifier string from initial auth request
///
/// # Returns
/// JSON string with format:
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
///
/// # Safety
/// Caller must free the returned string with `rust_free_string()`
#[no_mangle]
pub extern "C" fn rust_exchange_auth_code(
    locale_code: *const c_char,
    auth_code: *const c_char,
    device_serial: *const c_char,
    pkce_verifier: *const c_char,
) -> *mut c_char {
    let response = catch_panic(|| {
        let locale_code = c_str_to_string(locale_code)?;
        let auth_code = c_str_to_string(auth_code)?;
        let device_serial = c_str_to_string(device_serial)?;
        let pkce_verifier = c_str_to_string(pkce_verifier)?;

        let locale = crate::api::auth::Locale::from_country_code(&locale_code)
            .ok_or_else(|| crate::LibationError::InvalidInput(format!("Invalid locale: {}", locale_code)))?;

        let pkce = crate::api::auth::PkceChallenge {
            verifier: pkce_verifier,
            challenge: String::new(), // Not needed for exchange
            method: "S256".to_string(),
        };

        let result = RUNTIME.block_on(async {
            crate::api::auth::exchange_authorization_code(
                &locale,
                &auth_code,
                &device_serial,
                &pkce,
            ).await
        })?;

        Ok(success_response(result))
    });

    string_to_c_str(response)
}

/// Refresh access token using refresh token
///
/// # Arguments
/// * `locale_code` - Audible locale (e.g., "us", "uk", "de")
/// * `refresh_token` - Refresh token from previous authentication
/// * `device_serial` - 32-character hex device serial
///
/// # Returns
/// JSON string with format:
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
///
/// # Safety
/// Caller must free the returned string with `rust_free_string()`
#[no_mangle]
pub extern "C" fn rust_refresh_access_token(
    locale_code: *const c_char,
    refresh_token: *const c_char,
    device_serial: *const c_char,
) -> *mut c_char {
    let response = catch_panic(|| {
        let locale_code = c_str_to_string(locale_code)?;
        let refresh_token = c_str_to_string(refresh_token)?;
        let device_serial = c_str_to_string(device_serial)?;

        let locale = crate::api::auth::Locale::from_country_code(&locale_code)
            .ok_or_else(|| crate::LibationError::InvalidInput(format!("Invalid locale: {}", locale_code)))?;

        let result = RUNTIME.block_on(async {
            crate::api::auth::refresh_access_token(
                &locale,
                &refresh_token,
                &device_serial,
            ).await
        })?;

        Ok(success_response(result))
    });

    string_to_c_str(response)
}

/// Get activation bytes for DRM decryption
///
/// # Arguments
/// * `locale_code` - Audible locale (e.g., "us", "uk", "de")
/// * `access_token` - Valid access token
///
/// # Returns
/// JSON string with format:
/// ```json
/// {
///   "success": true,
///   "data": {
///     "activation_bytes": "1CEB00DA"
///   }
/// }
/// ```
///
/// # Safety
/// Caller must free the returned string with `rust_free_string()`
#[no_mangle]
pub extern "C" fn rust_get_activation_bytes(
    locale_code: *const c_char,
    access_token: *const c_char,
) -> *mut c_char {
    let response = catch_panic(|| {
        let locale_code = c_str_to_string(locale_code)?;
        let access_token = c_str_to_string(access_token)?;

        let locale = crate::api::auth::Locale::from_country_code(&locale_code)
            .ok_or_else(|| crate::LibationError::InvalidInput(format!("Invalid locale: {}", locale_code)))?;

        let result = RUNTIME.block_on(async {
            crate::api::auth::get_activation_bytes(&locale, &access_token).await
        })?;

        let response = serde_json::json!({
            "activation_bytes": result,
        });

        Ok(success_response(response))
    });

    string_to_c_str(response)
}

// ============================================================================
// DATABASE FUNCTIONS
// ============================================================================

/// Initialize database at specified path
///
/// # Arguments
/// * `db_path` - Absolute path to SQLite database file
///
/// # Returns
/// JSON string with format:
/// ```json
/// {
///   "success": true,
///   "data": {
///     "initialized": true
///   }
/// }
/// ```
///
/// # Safety
/// Caller must free the returned string with `rust_free_string()`
#[no_mangle]
pub extern "C" fn rust_init_database(db_path: *const c_char) -> *mut c_char {
    let response = catch_panic(|| {
        let db_path = c_str_to_string(db_path)?;

        let result = RUNTIME.block_on(async {
            let _db = crate::storage::Database::new(&db_path).await?;

            let response = serde_json::json!({
                "initialized": true,
            });

            Ok::<_, crate::LibationError>(response)
        })?;

        Ok(success_response(result))
    });

    string_to_c_str(response)
}

/// Synchronize library from Audible API
///
/// # Arguments
/// * `db_path` - Absolute path to SQLite database file
/// * `account_json` - JSON string containing serialized Account object
///
/// # Returns
/// JSON string with format:
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
///
/// # Safety
/// Caller must free the returned string with `rust_free_string()`
#[no_mangle]
pub extern "C" fn rust_sync_library(
    db_path: *const c_char,
    account_json: *const c_char,
) -> *mut c_char {
    let response = catch_panic(|| {
        let db_path = c_str_to_string(db_path)?;
        let account_json = c_str_to_string(account_json)?;

        let account: crate::api::auth::Account = serde_json::from_str(&account_json)
            .map_err(|e| crate::LibationError::InvalidInput(format!("Invalid account JSON: {}", e)))?;

        let result = RUNTIME.block_on(async {
            let db = crate::storage::Database::new(&db_path).await?;

            // Verify account has identity tokens
            account.identity.as_ref()
                .ok_or_else(|| crate::LibationError::AuthenticationFailed {
                    message: "No identity tokens".to_string(),
                    account_id: Some(account.account_id.clone()),
                })?;

            let mut client = crate::api::client::AudibleClient::new(account.clone())?;

            client.sync_library(&db, &account).await
        })?;

        Ok(success_response(result))
    });

    string_to_c_str(response)
}

/// Synchronize a single page of library from Audible API
///
/// This allows for progressive UI updates by fetching one page at a time.
///
/// # Arguments
/// * `db_path` - Absolute path to SQLite database file
/// * `account_json` - JSON-serialized Account object with identity
/// * `page` - Page number to fetch (1-indexed)
///
/// # Returns
/// JSON string with format:
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
///
/// # Safety
/// Caller must free the returned string with `rust_free_string()`
#[no_mangle]
pub extern "C" fn rust_sync_library_page(
    db_path: *const c_char,
    account_json: *const c_char,
    page: i32,
) -> *mut c_char {
    let response = catch_panic(|| {
        let db_path = c_str_to_string(db_path)?;
        let account_json = c_str_to_string(account_json)?;

        let account: crate::api::auth::Account = serde_json::from_str(&account_json)
            .map_err(|e| crate::LibationError::InvalidInput(format!("Invalid account JSON: {}", e)))?;

        let result = RUNTIME.block_on(async {
            let db = crate::storage::Database::new(&db_path).await?;

            // Verify account has identity tokens
            account.identity.as_ref()
                .ok_or_else(|| crate::LibationError::AuthenticationFailed {
                    message: "No identity tokens".to_string(),
                    account_id: Some(account.account_id.clone()),
                })?;

            let mut client = crate::api::client::AudibleClient::new(account.clone())?;

            client.sync_library_page(&db, &account, page).await
        })?;

        Ok(success_response(result))
    });

    string_to_c_str(response)
}

/// Get books from database with pagination
///
/// # Arguments
/// * `db_path` - Absolute path to SQLite database file
/// * `offset` - Number of books to skip
/// * `limit` - Maximum number of books to return
///
/// # Returns
/// JSON string with format:
/// ```json
/// {
///   "success": true,
///   "data": {
///     "books": [...],
///     "total_count": 150
///   }
/// }
/// ```
///
/// # Safety
/// Caller must free the returned string with `rust_free_string()`
#[no_mangle]
pub extern "C" fn rust_get_books(
    db_path: *const c_char,
    offset: i64,
    limit: i64,
) -> *mut c_char {
    let response = catch_panic(|| {
        let db_path = c_str_to_string(db_path)?;

        let result = RUNTIME.block_on(async {
            let db = crate::storage::Database::new(&db_path).await?;
            let books = crate::storage::queries::list_books(db.pool(), limit, offset).await?;
            let total_count = crate::storage::queries::count_books(db.pool()).await?;

            let response = serde_json::json!({
                "books": books,
                "total_count": total_count,
            });

            Ok::<_, crate::LibationError>(response)
        })?;

        Ok(success_response(result))
    });

    string_to_c_str(response)
}

/// Search books by title
///
/// # Arguments
/// * `db_path` - Absolute path to SQLite database file
/// * `query` - Search query string
///
/// # Returns
/// JSON string with format:
/// ```json
/// {
///   "success": true,
///   "data": {
///     "books": [...]
///   }
/// }
/// ```
///
/// # Safety
/// Caller must free the returned string with `rust_free_string()`
#[no_mangle]
pub extern "C" fn rust_search_books(
    db_path: *const c_char,
    query: *const c_char,
) -> *mut c_char {
    let response = catch_panic(|| {
        let db_path = c_str_to_string(db_path)?;
        let query = c_str_to_string(query)?;

        let result = RUNTIME.block_on(async {
            let db = crate::storage::Database::new(&db_path).await?;
            let books = crate::storage::queries::search_books_by_title(
                db.pool(),
                &query,
                50, // Default limit
            ).await?;

            let response = serde_json::json!({
                "books": books,
            });

            Ok::<_, crate::LibationError>(response)
        })?;

        Ok(success_response(result))
    });

    string_to_c_str(response)
}

// ============================================================================
// DOWNLOAD/DECRYPT FUNCTIONS
// ============================================================================

/// Download audiobook file
///
/// # Arguments
/// * `asin` - Amazon Standard Identification Number
/// * `access_token` - Valid access token
/// * `locale_code` - Audible locale (e.g., "us", "uk", "de")
/// * `output_path` - Absolute path where file should be saved
///
/// # Returns
/// JSON string with format:
/// ```json
/// {
///   "success": true,
///   "data": {
///     "bytes_downloaded": 123456789,
///     "output_path": "/path/to/book.aax"
///   }
/// }
/// ```
///
/// # Safety
/// Caller must free the returned string with `rust_free_string()`
#[no_mangle]
pub extern "C" fn rust_download_book(
    asin: *const c_char,
    access_token: *const c_char,
    locale_code: *const c_char,
    output_path: *const c_char,
) -> *mut c_char {
    let response = catch_panic(|| {
        let asin = c_str_to_string(asin)?;
        let access_token = c_str_to_string(access_token)?;
        let locale_code = c_str_to_string(locale_code)?;
        let output_path = c_str_to_string(output_path)?;

        let locale = crate::api::auth::Locale::from_country_code(&locale_code)
            .ok_or_else(|| crate::LibationError::InvalidInput(format!("Invalid locale: {}", locale_code)))?;

        let result = RUNTIME.block_on(async {
            // Create a minimal account for the client
            // Note: This is a simplified approach - in production you'd want the full Account object
            let account = crate::api::auth::Account {
                account_id: asin.clone(), // Use ASIN as temporary account ID
                account_name: "Temporary".to_string(),
                library_scan: false,
                decrypt_key: String::new(),
                identity: Some(crate::api::auth::Identity {
                    access_token: crate::api::auth::AccessToken {
                        token: access_token.clone(),
                        expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
                    },
                    refresh_token: String::new(),
                    device_private_key: String::new(),
                    adp_token: String::new(),
                    cookies: HashMap::new(),
                    device_serial_number: String::new(),
                    device_type: String::new(),
                    device_name: String::new(),
                    amazon_account_id: String::new(),
                    store_authentication_cookie: String::new(),
                    locale: locale,
                    customer_info: crate::api::auth::CustomerInfo {
                        account_pool: String::new(),
                        user_id: String::new(),
                        home_region: String::new(),
                        name: String::new(),
                        given_name: String::new(),
                    },
                }),
            };

            let client = crate::api::client::AudibleClient::new(account)?;
            let config = crate::download::manager::DownloadConfig::default();
            let download_manager = crate::download::DownloadManager::new(client, config);

            // Note: This is a placeholder - actual implementation would need license data
            // and content URL from the API
            let bytes_downloaded = 0u64; // TODO: Implement actual download

            let response = serde_json::json!({
                "bytes_downloaded": bytes_downloaded,
                "output_path": output_path,
            });

            Ok::<_, crate::LibationError>(response)
        })?;

        Ok(success_response(result))
    });

    string_to_c_str(response)
}

/// Decrypt AAX file to M4B using activation bytes
///
/// # Arguments
/// * `input_path` - Absolute path to input AAX file
/// * `output_path` - Absolute path where M4B file should be saved
/// * `activation_bytes` - 8-character hex activation bytes (e.g., "1CEB00DA")
///
/// # Returns
/// JSON string with format:
/// ```json
/// {
///   "success": true,
///   "data": {
///     "output_path": "/path/to/book.m4b",
///     "file_size": 123456789
///   }
/// }
/// ```
///
/// # Safety
/// Caller must free the returned string with `rust_free_string()`
#[no_mangle]
pub extern "C" fn rust_decrypt_aax(
    input_path: *const c_char,
    output_path: *const c_char,
    activation_bytes: *const c_char,
) -> *mut c_char {
    let response = catch_panic(|| {
        let input_path = c_str_to_string(input_path)?;
        let output_path = c_str_to_string(output_path)?;
        let activation_bytes = c_str_to_string(activation_bytes)?;

        let activation_bytes = crate::crypto::activation::ActivationBytes::from_hex(&activation_bytes)?;

        let result = RUNTIME.block_on(async {
            let decrypter = crate::crypto::aax::AaxDecrypter::new(activation_bytes);

            let input_path = std::path::Path::new(&input_path);
            let output_path = std::path::Path::new(&output_path);

            decrypter.decrypt_file(input_path, output_path).await?;

            let file_size = tokio::fs::metadata(output_path)
                .await
                .map(|m| m.len())
                .unwrap_or(0);

            let response = serde_json::json!({
                "output_path": output_path.to_string_lossy(),
                "file_size": file_size,
            });

            Ok::<_, crate::LibationError>(response)
        })?;

        Ok(success_response(result))
    });

    string_to_c_str(response)
}

// ============================================================================
// UTILITY FUNCTIONS
// ============================================================================

/// Validate activation bytes format
///
/// # Arguments
/// * `activation_bytes` - 8-character hex string to validate
///
/// # Returns
/// JSON string with format:
/// ```json
/// {
///   "success": true,
///   "data": {
///     "valid": true
///   }
/// }
/// ```
///
/// # Safety
/// Caller must free the returned string with `rust_free_string()`
#[no_mangle]
pub extern "C" fn rust_validate_activation_bytes(activation_bytes: *const c_char) -> *mut c_char {
    let response = catch_panic(|| {
        let activation_bytes = c_str_to_string(activation_bytes)?;

        let valid = crate::crypto::activation::ActivationBytes::from_hex(&activation_bytes).is_ok();

        let response = serde_json::json!({
            "valid": valid,
        });

        Ok(success_response(response))
    });

    string_to_c_str(response)
}

/// Get list of supported locales
///
/// # Returns
/// JSON string with format:
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
///
/// # Safety
/// Caller must free the returned string with `rust_free_string()`
#[no_mangle]
pub extern "C" fn rust_get_supported_locales() -> *mut c_char {
    let response = catch_panic(|| {
        let locales = crate::api::auth::Locale::all();

        let response = serde_json::json!({
            "locales": locales,
        });

        Ok(success_response(response))
    });

    string_to_c_str(response)
}

// ============================================================================
// MEMORY MANAGEMENT
// ============================================================================

/// Free a string pointer returned by Rust
///
/// # Arguments
/// * `ptr` - Pointer to C string allocated by Rust
///
/// # Safety
/// This function MUST be called exactly once for each string returned by
/// any other Rust function. Calling it multiple times on the same pointer
/// will cause a double-free error. Not calling it at all will cause a memory leak.
#[no_mangle]
pub extern "C" fn rust_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe {
            // Take ownership and drop the CString
            let _ = CString::from_raw(ptr);
        }
    }
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
        let result = catch_panic(|| Ok("normal result".to_string()));
        assert_eq!(result, "normal result");
    }

    #[test]
    fn test_catch_panic_with_panic() {
        let result = catch_panic(|| -> crate::Result<String> {
            panic!("test panic");
        });
        assert!(result.contains("\"success\":false"));
        assert!(result.contains("test panic"));
    }

    #[test]
    fn test_string_conversions() {
        let test_str = "Hello, World!";
        let c_str = CString::new(test_str).unwrap();
        let c_ptr = c_str.as_ptr();

        let rust_str = c_str_to_string(c_ptr).unwrap();
        assert_eq!(rust_str, test_str);
    }

    #[test]
    fn test_null_pointer_handling() {
        let result = c_str_to_string(std::ptr::null());
        assert!(result.is_err());
    }

    #[test]
    fn test_memory_safety() {
        // Test that we can create and free a string
        let test_str = "Memory test".to_string();
        let c_ptr = string_to_c_str(test_str);
        assert!(!c_ptr.is_null());

        // Free it
        rust_free_string(c_ptr);

        // Test that freeing null is safe
        rust_free_string(std::ptr::null_mut());
    }
}
