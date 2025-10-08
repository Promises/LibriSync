/**
 * C FFI Bridge for iOS - Rust Core Functionality
 *
 * This header file defines C-compatible FFI functions that expose Rust core
 * functionality to Swift/Objective-C code in Expo modules.
 *
 * IMPORTANT: All functions return JSON strings that MUST be freed with rust_free_string()
 *
 * JSON Response Format:
 * Success: {"success": true, "data": {...}}
 * Error:   {"success": false, "error": "error message"}
 *
 * Memory Management:
 * - All char* return values are owned by the caller
 * - Caller MUST call rust_free_string() on each returned pointer exactly once
 * - Failure to free will cause memory leaks
 * - Freeing twice will cause crashes
 *
 * Example Swift Usage:
 * ```swift
 * let resultPtr = rust_generate_oauth_url(locale, serial)
 * defer { rust_free_string(resultPtr) }
 * let jsonString = String(cString: resultPtr)
 * // Parse JSON and use data
 * ```
 */

#ifndef IOS_BRIDGE_H
#define IOS_BRIDGE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// ============================================================================
// AUTHENTICATION FUNCTIONS
// ============================================================================

/**
 * Generate OAuth authorization URL for Audible login
 *
 * @param locale_code Audible locale (e.g., "us", "uk", "de")
 * @param device_serial 32-character hex device serial
 * @return JSON string with authorization_url, pkce_verifier, and state
 *         Caller must free with rust_free_string()
 */
char* rust_generate_oauth_url(const char* locale_code, const char* device_serial);

/**
 * Parse OAuth callback URL to extract authorization code
 *
 * @param callback_url Full callback URL with authorization code
 * @return JSON string with authorization_code
 *         Caller must free with rust_free_string()
 */
char* rust_parse_oauth_callback(const char* callback_url);

/**
 * Exchange authorization code for access and refresh tokens
 *
 * @param locale_code Audible locale (e.g., "us", "uk", "de")
 * @param auth_code Authorization code from OAuth callback
 * @param device_serial 32-character hex device serial
 * @param pkce_verifier PKCE verifier string from initial auth request
 * @return JSON string with access_token, refresh_token, expires_in, token_type
 *         Caller must free with rust_free_string()
 */
char* rust_exchange_auth_code(
    const char* locale_code,
    const char* auth_code,
    const char* device_serial,
    const char* pkce_verifier
);

/**
 * Refresh access token using refresh token
 *
 * @param locale_code Audible locale (e.g., "us", "uk", "de")
 * @param refresh_token Refresh token from previous authentication
 * @param device_serial 32-character hex device serial
 * @return JSON string with access_token, refresh_token, expires_in, token_type
 *         Caller must free with rust_free_string()
 */
char* rust_refresh_access_token(
    const char* locale_code,
    const char* refresh_token,
    const char* device_serial
);

/**
 * Get activation bytes for DRM decryption
 *
 * @param locale_code Audible locale (e.g., "us", "uk", "de")
 * @param access_token Valid access token
 * @return JSON string with activation_bytes (8-character hex string)
 *         Caller must free with rust_free_string()
 */
char* rust_get_activation_bytes(
    const char* locale_code,
    const char* access_token
);

// ============================================================================
// DATABASE FUNCTIONS
// ============================================================================

/**
 * Initialize database at specified path
 *
 * @param db_path Absolute path to SQLite database file
 * @return JSON string with initialized: true
 *         Caller must free with rust_free_string()
 */
char* rust_init_database(const char* db_path);

/**
 * Synchronize library from Audible API
 *
 * @param db_path Absolute path to SQLite database file
 * @param account_json JSON string containing serialized Account object
 * @return JSON string with sync statistics (total_items, books_added, books_updated, etc.)
 *         Caller must free with rust_free_string()
 */
char* rust_sync_library(const char* db_path, const char* account_json);

/**
 * Get books from database with pagination
 *
 * @param db_path Absolute path to SQLite database file
 * @param offset Number of books to skip
 * @param limit Maximum number of books to return
 * @return JSON string with books array and total_count
 *         Caller must free with rust_free_string()
 */
char* rust_get_books(const char* db_path, int64_t offset, int64_t limit);

/**
 * Search books by title
 *
 * @param db_path Absolute path to SQLite database file
 * @param query Search query string
 * @return JSON string with books array
 *         Caller must free with rust_free_string()
 */
char* rust_search_books(const char* db_path, const char* query);

// ============================================================================
// DOWNLOAD/DECRYPT FUNCTIONS
// ============================================================================

/**
 * Download audiobook file
 *
 * @param asin Amazon Standard Identification Number
 * @param access_token Valid access token
 * @param locale_code Audible locale (e.g., "us", "uk", "de")
 * @param output_path Absolute path where file should be saved
 * @return JSON string with bytes_downloaded and output_path
 *         Caller must free with rust_free_string()
 */
char* rust_download_book(
    const char* asin,
    const char* access_token,
    const char* locale_code,
    const char* output_path
);

/**
 * Decrypt AAX file to M4B using activation bytes
 *
 * @param input_path Absolute path to input AAX file
 * @param output_path Absolute path where M4B file should be saved
 * @param activation_bytes 8-character hex activation bytes (e.g., "1CEB00DA")
 * @return JSON string with output_path and file_size
 *         Caller must free with rust_free_string()
 */
char* rust_decrypt_aax(
    const char* input_path,
    const char* output_path,
    const char* activation_bytes
);

// ============================================================================
// UTILITY FUNCTIONS
// ============================================================================

/**
 * Validate activation bytes format
 *
 * @param activation_bytes 8-character hex string to validate
 * @return JSON string with valid: true/false
 *         Caller must free with rust_free_string()
 */
char* rust_validate_activation_bytes(const char* activation_bytes);

/**
 * Get list of supported locales
 *
 * @return JSON string with locales array containing country_code, name, and domain
 *         Caller must free with rust_free_string()
 */
char* rust_get_supported_locales(void);

// ============================================================================
// MEMORY MANAGEMENT
// ============================================================================

/**
 * Free a string pointer returned by Rust
 *
 * CRITICAL: This function MUST be called exactly once for each string returned
 * by any other Rust function. Calling it multiple times on the same pointer
 * will cause a double-free error. Not calling it at all will cause a memory leak.
 *
 * @param ptr Pointer to C string allocated by Rust (can be NULL, which is safe)
 */
void rust_free_string(char* ptr);

#ifdef __cplusplus
}
#endif

#endif /* IOS_BRIDGE_H */
