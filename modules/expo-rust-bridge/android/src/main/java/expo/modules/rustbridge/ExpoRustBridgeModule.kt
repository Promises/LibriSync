package expo.modules.rustbridge

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import expo.modules.kotlin.Promise
import org.json.JSONObject
import org.json.JSONArray

class ExpoRustBridgeModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("ExpoRustBridge")

    // ============================================================================
    // AUTHENTICATION FUNCTIONS
    // ============================================================================

    /**
     * Generate an OAuth authorization URL for Audible login.
     *
     * @param localeCode The Audible locale (e.g., "us", "uk", "de")
     * @param deviceSerial The device serial number (32 hex characters)
     * @return Map with success flag and either data (url, pkce, state) or error message
     */
    Function("generateOAuthUrl") { localeCode: String, deviceSerial: String ->
      val params = JSONObject().apply {
        put("locale_code", localeCode)
        put("device_serial", deviceSerial)
      }
      parseJsonResponse(nativeGenerateOAuthUrl(params.toString()))
    }

    /**
     * Parse the OAuth callback URL to extract authorization code.
     *
     * @param callbackUrl The callback URL received from Audible OAuth
     * @return Map with success flag and either data (auth_code, state) or error message
     */
    Function("parseOAuthCallback") { callbackUrl: String ->
      val params = JSONObject().apply {
        put("callback_url", callbackUrl)
      }
      parseJsonResponse(nativeParseOAuthCallback(params.toString()))
    }

    /**
     * Exchange authorization code for access and refresh tokens.
     *
     * @param localeCode The Audible locale
     * @param authCode The authorization code from callback
     * @param deviceSerial The device serial number
     * @param pkceVerifier The PKCE code verifier from initial OAuth request
     * @return Promise resolving to Map with tokens or rejecting with error
     */
    AsyncFunction("exchangeAuthCode") { localeCode: String, authCode: String, deviceSerial: String, pkceVerifier: String ->
      try {
        val params = JSONObject().apply {
          put("locale_code", localeCode)
          put("authorization_code", authCode)
          put("device_serial", deviceSerial)
          put("pkce_verifier", pkceVerifier)
        }
        val result = nativeExchangeAuthCode(params.toString())
        parseJsonResponse(result)
      } catch (e: Exception) {
        mapOf(
          "success" to false,
          "error" to "Exchange auth code error: ${e.message}"
        )
      }
    }

    /**
     * Refresh an expired access token using the refresh token.
     *
     * @param localeCode The Audible locale
     * @param refreshToken The refresh token
     * @param deviceSerial The device serial number
     * @return Promise resolving to Map with new tokens or rejecting with error
     */
    AsyncFunction("refreshAccessToken") { localeCode: String, refreshToken: String, deviceSerial: String ->
      try {
        val params = JSONObject().apply {
          put("locale_code", localeCode)
          put("refresh_token", refreshToken)
          put("device_serial", deviceSerial)
        }
        val result = nativeRefreshAccessToken(params.toString())
        parseJsonResponse(result)
      } catch (e: Exception) {
        mapOf(
          "success" to false,
          "error" to "Refresh token error: ${e.message}"
        )
      }
    }

    /**
     * Get activation bytes for DRM removal using access token.
     *
     * @param localeCode The Audible locale
     * @param accessToken The access token
     * @return Promise resolving to Map with activation bytes or rejecting with error
     */
    AsyncFunction("getActivationBytes") { localeCode: String, accessToken: String ->
      try {
        val params = JSONObject().apply {
          put("locale_code", localeCode)
          put("access_token", accessToken)
        }
        val result = nativeGetActivationBytes(params.toString())
        parseJsonResponse(result)
      } catch (e: Exception) {
        mapOf(
          "success" to false,
          "error" to "Get activation bytes error: ${e.message}"
        )
      }
    }

    // ============================================================================
    // DATABASE FUNCTIONS
    // ============================================================================

    /**
     * Initialize the SQLite database with schema.
     *
     * @param dbPath The path to the SQLite database file
     * @return Map with success flag and error message if failed
     */
    Function("initDatabase") { dbPath: String ->
      val params = JSONObject().apply {
        put("db_path", dbPath)
      }
      parseJsonResponse(nativeInitDatabase(params.toString()))
    }

    /**
     * Sync library from Audible API to local database.
     *
     * @param dbPath The path to the SQLite database file
     * @param accountJson JSON string containing account info (access_token, locale, etc.)
     * @return Promise resolving to Map with sync results or rejecting with error
     */
    AsyncFunction("syncLibrary") { dbPath: String, accountJson: String ->
      try {
        val params = JSONObject().apply {
          put("db_path", dbPath)
          put("account_json", accountJson)
        }
        val result = nativeSyncLibrary(params.toString())
        parseJsonResponse(result)
      } catch (e: Exception) {
        mapOf(
          "success" to false,
          "error" to "Sync library error: ${e.message}"
        )
      }
    }

    /**
     * Sync a single page of library from Audible API.
     *
     * This allows for progressive UI updates by fetching one page at a time.
     *
     * @param dbPath The path to the SQLite database file
     * @param accountJson JSON string containing account info (access_token, locale, etc.)
     * @param page The page number to fetch (1-indexed)
     * @return Promise resolving to Map with sync results including has_more flag
     */
    AsyncFunction("syncLibraryPage") { dbPath: String, accountJson: String, page: Int ->
      try {
        val params = JSONObject().apply {
          put("db_path", dbPath)
          put("account_json", accountJson)
          put("page", page)
        }
        val result = nativeSyncLibraryPage(params.toString())
        parseJsonResponse(result)
      } catch (e: Exception) {
        mapOf(
          "success" to false,
          "error" to "Sync library page error: ${e.message}"
        )
      }
    }

    /**
     * Get paginated list of books from database.
     *
     * @param dbPath The path to the SQLite database file
     * @param offset The pagination offset
     * @param limit The number of books to retrieve
     * @return Map with success flag and list of books or error message
     */
    Function("getBooks") { dbPath: String, offset: Int, limit: Int ->
      val params = JSONObject().apply {
        put("db_path", dbPath)
        put("offset", offset)
        put("limit", limit)
      }
      parseJsonResponse(nativeGetBooks(params.toString()))
    }

    /**
     * Search books in database by title, author, or narrator.
     *
     * @param dbPath The path to the SQLite database file
     * @param query The search query string
     * @return Map with success flag and list of matching books or error message
     */
    Function("searchBooks") { dbPath: String, query: String ->
      val params = JSONObject().apply {
        put("db_path", dbPath)
        put("query", query)
      }
      parseJsonResponse(nativeSearchBooks(params.toString()))
    }

    // ============================================================================
    // DOWNLOAD & DECRYPTION FUNCTIONS
    // ============================================================================

    /**
     * Download an audiobook from Audible.
     *
     * @param asin The Amazon Standard Identification Number
     * @param licenseJson JSON string containing license/token info
     * @param outputPath The path where the .aax file should be saved
     * @return Promise resolving to Map with download info or rejecting with error
     */
    AsyncFunction("downloadBook") { asin: String, licenseJson: String, outputPath: String ->
      try {
        val params = JSONObject().apply {
          put("asin", asin)
          put("license_json", licenseJson)
          put("output_path", outputPath)
        }
        val result = nativeDownloadBook(params.toString())
        parseJsonResponse(result)
      } catch (e: Exception) {
        mapOf(
          "success" to false,
          "error" to "Download book error: ${e.message}"
        )
      }
    }

    /**
     * Decrypt an AAX file to M4B using activation bytes.
     *
     * @param inputPath The path to the encrypted .aax file
     * @param outputPath The path where the decrypted .m4b file should be saved
     * @param activationBytes The activation bytes for DRM removal
     * @return Promise resolving to Map with decryption info or rejecting with error
     */
    AsyncFunction("decryptAAX") { inputPath: String, outputPath: String, activationBytes: String ->
      try {
        val params = JSONObject().apply {
          put("input_path", inputPath)
          put("output_path", outputPath)
          put("activation_bytes", activationBytes)
        }
        val result = nativeDecryptAAX(params.toString())
        parseJsonResponse(result)
      } catch (e: Exception) {
        mapOf(
          "success" to false,
          "error" to "Decrypt AAX error: ${e.message}"
        )
      }
    }

    // ============================================================================
    // UTILITY FUNCTIONS
    // ============================================================================

    /**
     * Validate activation bytes format (8 hex bytes).
     *
     * @param activationBytes The activation bytes string to validate
     * @return Map with success flag and validation result or error message
     */
    Function("validateActivationBytes") { activationBytes: String ->
      val params = JSONObject().apply {
        put("activation_bytes", activationBytes)
      }
      parseJsonResponse(nativeValidateActivationBytes(params.toString()))
    }

    /**
     * Get list of supported Audible locales.
     *
     * @return Map with success flag and array of supported locales or error message
     */
    Function("getSupportedLocales") {
      val params = JSONObject() // Empty params
      parseJsonResponse(nativeGetSupportedLocales(params.toString()))
    }

    /**
     * Get customer information from Audible API.
     *
     * @param localeCode The Audible locale (e.g., "us", "uk")
     * @param accessToken Valid access token
     * @return Map with success flag and customer info (name, email) or error message
     */
    AsyncFunction("getCustomerInformation") { localeCode: String, accessToken: String, promise: Promise ->
      val params = JSONObject().apply {
        put("locale_code", localeCode)
        put("access_token", accessToken)
      }
      val response = parseJsonResponse(nativeGetCustomerInformation(params.toString()))
      promise.resolve(response)
    }

    /**
     * Test bridge connection and verify Rust library is loaded.
     *
     * @return Map with bridge status information
     */
    Function("testBridge") {
      mapOf(
        "bridgeActive" to true,
        "rustLoaded" to true,
        "version" to "0.1.0"
      )
    }

    /**
     * Legacy test function - logs a message from Rust.
     *
     * @param message The message to log
     * @return The response from Rust
     */
    Function("logFromRust") { message: String ->
      val params = JSONObject().apply {
        put("message", message)
      }
      parseJsonResponse(nativeLogFromRust(params.toString()))
    }
  }

  // ============================================================================
  // NATIVE METHOD DECLARATIONS (JNI Bridge)
  // ============================================================================

  // All native methods accept a single JSON string parameter
  private external fun nativeGenerateOAuthUrl(paramsJson: String): String
  private external fun nativeParseOAuthCallback(paramsJson: String): String
  private external fun nativeExchangeAuthCode(paramsJson: String): String
  private external fun nativeRefreshAccessToken(paramsJson: String): String
  private external fun nativeGetActivationBytes(paramsJson: String): String
  private external fun nativeInitDatabase(paramsJson: String): String
  private external fun nativeSyncLibrary(paramsJson: String): String
  private external fun nativeSyncLibraryPage(paramsJson: String): String
  private external fun nativeGetBooks(paramsJson: String): String
  private external fun nativeSearchBooks(paramsJson: String): String
  private external fun nativeDownloadBook(paramsJson: String): String
  private external fun nativeDecryptAAX(paramsJson: String): String
  private external fun nativeValidateActivationBytes(paramsJson: String): String
  private external fun nativeGetSupportedLocales(paramsJson: String): String
  private external fun nativeGetCustomerInformation(paramsJson: String): String
  private external fun nativeLogFromRust(paramsJson: String): String

  // ============================================================================
  // JSON PARSING HELPERS
  // ============================================================================

  /**
   * Parse JSON response from Rust into a Kotlin Map.
   *
   * Rust returns JSON in the format:
   * Success: { "success": true, "data": {...} }
   * Error: { "success": false, "error": "error message" }
   *
   * @param jsonString The JSON string from Rust
   * @return Map with success flag and either data or error
   */
  private fun parseJsonResponse(jsonString: String): Map<String, Any?> {
    return try {
      val json = JSONObject(jsonString)
      val success = json.getBoolean("success")

      if (success) {
        mapOf(
          "success" to true,
          "data" to parseJsonValue(json.get("data"))
        )
      } else {
        mapOf(
          "success" to false,
          "error" to json.getString("error")
        )
      }
    } catch (e: Exception) {
      mapOf(
        "success" to false,
        "error" to "Failed to parse JSON response: ${e.message}"
      )
    }
  }

  /**
   * Recursively parse JSON values into Kotlin types.
   *
   * @param value The JSON value to parse
   * @return Kotlin representation (Map, List, or primitive)
   */
  private fun parseJsonValue(value: Any?): Any? {
    return when (value) {
      is JSONObject -> {
        val map = mutableMapOf<String, Any?>()
        value.keys().forEach { key ->
          map[key] = parseJsonValue(value.get(key))
        }
        map
      }
      is JSONArray -> {
        (0 until value.length()).map { i ->
          parseJsonValue(value.get(i))
        }
      }
      JSONObject.NULL -> null
      else -> value
    }
  }

  // ============================================================================
  // NATIVE LIBRARY LOADING
  // ============================================================================

  companion object {
    init {
      try {
        System.loadLibrary("rust_core")
        android.util.Log.i("ExpoRustBridge", "Successfully loaded rust_core library")
      } catch (e: UnsatisfiedLinkError) {
        // Library not found - this is expected in development mode
        // until Rust library is built
        android.util.Log.w("ExpoRustBridge", "Failed to load rust_core library: ${e.message}")
      }
    }
  }
}
