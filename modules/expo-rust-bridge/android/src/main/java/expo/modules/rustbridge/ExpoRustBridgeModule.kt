package expo.modules.rustbridge

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import expo.modules.kotlin.Promise
import org.json.JSONObject
import org.json.JSONArray
import android.net.Uri
import androidx.documentfile.provider.DocumentFile

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
    /**
     * Download and decrypt an audiobook (complete pipeline with FFmpeg-Kit).
     *
     * Flow:
     * 1. Rust downloads encrypted .aax to cache and returns decryption keys
     * 2. Kotlin uses FFmpeg-Kit to decrypt to .m4b in cache
     * 3. If SAF URI, Kotlin copies to user's directory using DocumentFile
     * 4. Returns final output path
     *
     * @param accountJson The complete account JSON with identity
     * @param asin The book ASIN to download
     * @param outputDirectory The directory to save the decrypted M4B file (can be SAF URI)
     * @param quality Download quality ("Low", "Normal", "High", "Extreme")
     * @return Promise resolving to Map with outputPath, fileSize, duration
     */
    AsyncFunction("downloadBook") { accountJson: String, asin: String, outputDirectory: String, quality: String ->
      try {
        val context = appContext.reactContext ?: throw Exception("React context not available")

        // Step 1: Download encrypted file using Rust
        val params = JSONObject().apply {
          put("accountJson", accountJson)
          put("asin", asin)
          put("outputDirectory", outputDirectory) // Ignored by Rust (uses cache)
          put("quality", quality)
        }

        val downloadResult = nativeDownloadBook(params.toString())
        val parsedResult = parseJsonResponse(downloadResult)

        if (parsedResult["success"] != true) {
          return@AsyncFunction parsedResult
        }

        val data = parsedResult["data"] as? Map<*, *>
          ?: return@AsyncFunction mapOf("success" to false, "error" to "Invalid download response")

        val encryptedPath = data["encryptedPath"] as? String
          ?: return@AsyncFunction mapOf("success" to false, "error" to "Missing encryptedPath")
        val decryptedCachePath = data["outputPath"] as? String
          ?: return@AsyncFunction mapOf("success" to false, "error" to "Missing outputPath")
        val aaxcKey = data["aaxcKey"] as? String
        val aaxcIv = data["aaxcIv"] as? String

        // Step 2: Decrypt using FFmpeg-Kit
        val command = buildList {
          add("-y") // Overwrite output

          if (aaxcKey != null && aaxcIv != null) {
            add("-audible_key")
            add(aaxcKey)
            add("-audible_iv")
            add(aaxcIv)
          }

          add("-i")
          add(encryptedPath)
          add("-c")
          add("copy")
          add("-vn")
          add(decryptedCachePath)
        }.toTypedArray()

        val session = com.arthenica.ffmpegkit.FFmpegKit.execute(command.joinToString(" "))

        if (!com.arthenica.ffmpegkit.ReturnCode.isSuccess(session.returnCode)) {
          return@AsyncFunction mapOf(
            "success" to false,
            "error" to "FFmpeg decryption failed: ${session.failStackTrace}"
          )
        }

        // Step 3: Get duration
        val infoSession = com.arthenica.ffmpegkit.FFprobeKit.getMediaInformation(decryptedCachePath)
        val duration = infoSession.mediaInformation?.duration?.toDoubleOrNull() ?: 0.0

        val cachedFile = java.io.File(decryptedCachePath)
        var finalOutputPath = decryptedCachePath
        var finalFileSize = cachedFile.length()

        // Step 4: If SAF URI, copy to user's directory using DocumentFile
        if (outputDirectory.startsWith("content://")) {
          val treeUri = Uri.parse(outputDirectory)
          val docDir = DocumentFile.fromTreeUri(context, treeUri)
            ?: throw Exception("Invalid SAF URI: $outputDirectory")

          if (!docDir.canWrite()) {
            throw Exception("No write permission for SAF directory: $outputDirectory")
          }

          val fileName = "$asin.m4b"

          // Delete existing file if present
          val existingFile = docDir.findFile(fileName)
          if (existingFile != null) {
            android.util.Log.d("ExpoRustBridge", "Deleting existing file: ${existingFile.uri}")
            existingFile.delete()
          }

          // Create new file - try multiple MIME types
          val outputFile = docDir.createFile("audio/mp4", fileName)
            ?: docDir.createFile("audio/x-m4b", fileName)
            ?: docDir.createFile("audio/*", fileName)
            ?: throw Exception("Failed to create file '$fileName' in SAF directory (tried multiple MIME types)")

          android.util.Log.d("ExpoRustBridge", "Created SAF file: ${outputFile.uri}")

          // Copy to SAF location
          context.contentResolver.openOutputStream(outputFile.uri)?.use { outputStream ->
            cachedFile.inputStream().use { inputStream ->
              inputStream.copyTo(outputStream)
            }
          } ?: throw Exception("Failed to open SAF output stream")

          finalOutputPath = outputFile.uri.toString()
          android.util.Log.d("ExpoRustBridge", "Copied to SAF: $finalOutputPath")

          // Delete cache file
          cachedFile.delete()
        }

        // Step 5: Delete encrypted file
        java.io.File(encryptedPath).delete()

        mapOf(
          "success" to true,
          "data" to mapOf(
            "outputPath" to finalOutputPath,
            "fileSize" to finalFileSize,
            "duration" to duration
          )
        )
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

    // ============================================================================
    // FFMPEG-KIT FUNCTIONS (16KB Page Size Compatible)
    // ============================================================================

    /**
     * Convert AAX/AAXC to M4B using FFmpeg-Kit.
     *
     * @param inputPath Path to input AAX/AAXC file
     * @param outputPath Path to output M4B file
     * @param activationBytes Optional activation bytes for AAX (8 hex chars)
     * @param aaxcKey Optional AAXC decryption key (hex string)
     * @param aaxcIv Optional AAXC initialization vector (hex string)
     * @return Promise resolving to Map with conversion info or error
     */
    AsyncFunction("convertToM4b") { inputPath: String, outputPath: String, activationBytes: String?, aaxcKey: String?, aaxcIv: String? ->
      try {
        val command = buildList {
          add("-y") // Overwrite output

          // Add decryption parameters
          when {
            aaxcKey != null && aaxcIv != null -> {
              add("-audible_key")
              add(aaxcKey)
              add("-audible_iv")
              add(aaxcIv)
            }
            activationBytes != null && activationBytes.isNotEmpty() -> {
              add("-activation_bytes")
              add(activationBytes)
            }
          }

          add("-i")
          add(inputPath)
          add("-c")
          add("copy") // Fast copy without re-encoding
          add("-vn") // No video
          add(outputPath)
        }.toTypedArray()

        val session = com.arthenica.ffmpegkit.FFmpegKit.execute(command.joinToString(" "))

        if (com.arthenica.ffmpegkit.ReturnCode.isSuccess(session.returnCode)) {
          val outputFile = java.io.File(outputPath)
          mapOf(
            "success" to true,
            "data" to mapOf(
              "outputPath" to outputPath,
              "fileSize" to outputFile.length(),
              "returnCode" to session.returnCode.value
            )
          )
        } else {
          mapOf(
            "success" to false,
            "error" to "FFmpeg conversion failed: ${session.failStackTrace}"
          )
        }
      } catch (e: Exception) {
        mapOf(
          "success" to false,
          "error" to "Convert to M4B error: ${e.message}"
        )
      }
    }

    /**
     * Convert audio file to different format using FFmpeg-Kit.
     *
     * @param inputPath Path to input file
     * @param outputPath Path to output file
     * @param codec Audio codec (aac, libmp3lame, copy)
     * @param bitrate Bitrate (e.g., "128k" or null for default)
     * @param quality VBR quality (0-9 for MP3, null for CBR)
     * @return Promise resolving to Map with conversion info or error
     */
    AsyncFunction("convertAudio") { inputPath: String, outputPath: String, codec: String, bitrate: String?, quality: Int? ->
      try {
        val command = buildList {
          add("-y") // Overwrite output
          add("-i")
          add(inputPath)
          add("-codec:a")
          add(codec)

          if (quality != null && codec == "libmp3lame") {
            add("-q:a")
            add(quality.toString())
          } else if (bitrate != null) {
            add("-b:a")
            add(bitrate)
          }

          add("-vn") // No video
          add(outputPath)
        }.toTypedArray()

        val session = com.arthenica.ffmpegkit.FFmpegKit.execute(command.joinToString(" "))

        if (com.arthenica.ffmpegkit.ReturnCode.isSuccess(session.returnCode)) {
          val outputFile = java.io.File(outputPath)
          mapOf(
            "success" to true,
            "data" to mapOf(
              "outputPath" to outputPath,
              "fileSize" to outputFile.length()
            )
          )
        } else {
          mapOf(
            "success" to false,
            "error" to "FFmpeg conversion failed: ${session.failStackTrace}"
          )
        }
      } catch (e: Exception) {
        mapOf(
          "success" to false,
          "error" to "Convert audio error: ${e.message}"
        )
      }
    }

    /**
     * Get audio file duration and metadata using FFprobe.
     *
     * @param filePath Path to audio file
     * @return Promise resolving to Map with duration and metadata
     */
    AsyncFunction("getAudioInfo") { filePath: String ->
      try {
        val session = com.arthenica.ffmpegkit.FFprobeKit.getMediaInformation(filePath)
        val info = session.mediaInformation

        if (info != null) {
          mapOf(
            "success" to true,
            "data" to mapOf(
              "duration" to info.duration.toDoubleOrNull(),
              "bitrate" to info.bitrate,
              "format" to info.format,
              "size" to info.size
            )
          )
        } else {
          mapOf(
            "success" to false,
            "error" to "Could not get media information"
          )
        }
      } catch (e: Exception) {
        mapOf(
          "success" to false,
          "error" to "Get audio info error: ${e.message}"
        )
      }
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
