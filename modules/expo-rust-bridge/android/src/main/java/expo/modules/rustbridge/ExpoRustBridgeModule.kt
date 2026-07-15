package expo.modules.rustbridge

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import expo.modules.kotlin.Promise
import org.json.JSONObject
import org.json.JSONArray
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Typeface
import android.net.Uri
import android.provider.DocumentsContract
import androidx.documentfile.provider.DocumentFile
import expo.modules.rustbridge.workers.WorkerScheduler
import java.io.File
import java.io.FileOutputStream
import java.net.URL
import kotlin.math.ceil
import kotlin.math.max
import kotlin.math.min

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
     * Sync one page of episodes for a podcast or periodical parent.
     */
    AsyncFunction("syncPodcastEpisodes") {
      dbPath: String,
      accountJson: String,
      parentAsin: String,
      offset: Int,
      limit: Int
    ->
      try {
        val params = JSONObject().apply {
          put("db_path", dbPath)
          put("account_json", accountJson)
          put("parent_asin", parentAsin)
          put("offset", offset)
          put("limit", limit)
        }
        val result = nativeSyncPodcastEpisodes(params.toString())
        parseJsonResponse(result)
      } catch (e: Exception) {
        mapOf(
          "success" to false,
          "error" to "Sync podcast episodes error: ${e.message}"
        )
      }
    }

    /**
     * Persist the user-selected download directory for background sync workers.
     */
    Function("setDownloadDirectory") { directory: String ->
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        ExistingDownloadScanner.saveDownloadDirectory(context, directory)
        mapOf("success" to true, "data" to mapOf("saved" to true))
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Read the native copy of the user-selected download directory.
     */
    Function("getDownloadDirectory") {
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        mapOf(
          "success" to true,
          "data" to mapOf("directory" to ExistingDownloadScanner.getSavedDownloadDirectory(context))
        )
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Scan the download directory for existing audio files and link matches to synced books.
     */
    AsyncFunction("scanDownloadDirectory") { dbPath: String, downloadDirectory: String? ->
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        val directory = downloadDirectory
          ?: ExistingDownloadScanner.getSavedDownloadDirectory(context)
          ?: throw Exception("Download directory is not set")

        ExistingDownloadScanner.saveDownloadDirectory(context, directory)
        val scanResult = ExistingDownloadScanner.scan(context, dbPath, directory)
        mapOf("success" to true, "data" to scanResult.toMap())
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
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

    /**
     * Get books with advanced filtering, sorting, and search.
     *
     * @param dbPath The path to the SQLite database file
     * @param offset Pagination offset
     * @param limit Maximum number of results
     * @param searchQuery Optional search query (searches title, author, narrator)
     * @param seriesName Optional series filter
     * @param category Optional category/genre filter
     * @param sortField Sort field: "title", "release_date", "date_added", "series", "length", or "downloaded"
     * @param extras JSON with sort direction, source, and optional downloaded group sort
     * @return Map with success flag, books array, and total_count
     */
    Function("getBooksWithFilters") {
      dbPath: String,
      offset: Int,
      limit: Int,
      searchQuery: String?,
      seriesName: String?,
      category: String?,
      sortField: String?,
      extras: String?
    ->
      val params = JSONObject().apply {
        put("db_path", dbPath)
        put("offset", offset)
        put("limit", limit)
        if (searchQuery != null) put("search_query", searchQuery)
        if (seriesName != null) put("series_name", seriesName)
        if (category != null) put("category", category)
        if (sortField != null) put("sort_field", sortField)
        // extras is a JSON string with optional sort_direction, source, and downloaded group sort
        if (extras != null) {
          try {
            val extrasObj = JSONObject(extras)
            if (extrasObj.has("sort_direction")) put("sort_direction", extrasObj.getString("sort_direction"))
            if (extrasObj.has("source")) put("source", extrasObj.getString("source"))
            if (extrasObj.has("downloaded_group_sort_field")) put("downloaded_group_sort_field", extrasObj.getString("downloaded_group_sort_field"))
            if (extrasObj.has("downloaded_group_sort_direction")) put("downloaded_group_sort_direction", extrasObj.getString("downloaded_group_sort_direction"))
            if (extrasObj.has("account")) put("account", extrasObj.getString("account"))
            if (extrasObj.has("origin_asin")) put("origin_asin", extrasObj.getString("origin_asin"))
            if (extrasObj.has("include_podcasts")) put("include_podcasts", extrasObj.getBoolean("include_podcasts"))
            if (extrasObj.has("podcasts_only")) put("podcasts_only", extrasObj.getBoolean("podcasts_only"))
            // Multi-select filters (JSON arrays of strings)
            if (extrasObj.has("series_names")) put("series_names", extrasObj.getJSONArray("series_names"))
            if (extrasObj.has("categories")) put("categories", extrasObj.getJSONArray("categories"))
            if (extrasObj.has("accounts")) put("accounts", extrasObj.getJSONArray("accounts"))
          } catch (_: Exception) {
            // If extras is not valid JSON, treat it as sort_direction for backward compat
            put("sort_direction", extras)
          }
        }
      }
      parseJsonResponse(nativeGetBooksWithFilters(params.toString()))
    }

    /**
     * Get all unique series names from the library.
     *
     * @param dbPath The path to the SQLite database file
     * @return Map with success flag and array of series names
     */
    Function("getAllSeries") { dbPath: String ->
      val params = JSONObject().apply {
        put("db_path", dbPath)
      }
      parseJsonResponse(nativeGetAllSeries(params.toString()))
    }

    /**
     * Export the library database as a single consistent SQLite file.
     *
     * @param dbPath The path to the SQLite database file
     * @param destPath Destination file path for the exported copy
     * @return Map with success flag and dest_path
     */
    Function("exportDatabase") { dbPath: String, destPath: String ->
      val params = JSONObject().apply {
        put("db_path", dbPath)
        put("dest_path", destPath)
      }
      parseJsonResponse(nativeExportDatabase(params.toString()))
    }

    /**
     * Get all unique categories/genres from the library.
     *
     * @param dbPath The path to the SQLite database file
     * @return Map with success flag and array of category names
     */
    Function("getAllCategories") { dbPath: String ->
      val params = JSONObject().apply {
        put("db_path", dbPath)
      }
      parseJsonResponse(nativeGetAllCategories(params.toString()))
    }

    /**
     * Render library export rows as a PNG file.
     */
    AsyncFunction("createLibraryExportImage") { entriesJson: String, outputUri: String, promise: Promise ->
      Thread {
        try {
          createLibraryExportImageFile(entriesJson, outputUri)
          promise.resolve(mapOf(
            "success" to true,
            "data" to mapOf("uri" to outputUri)
          ))
        } catch (e: Exception) {
          promise.resolve(mapOf(
            "success" to false,
            "error" to "Create library export image error: ${e.message}"
          ))
        }
      }.start()
    }

    /**
     * Copy text to the system clipboard.
     */
    Function("copyTextToClipboard") { text: String ->
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("LibriSync Library Export", text))
        mapOf(
          "success" to true,
          "data" to mapOf("copied" to true)
        )
      } catch (e: Exception) {
        mapOf(
          "success" to false,
          "error" to "Copy text to clipboard error: ${e.message}"
        )
      }
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

    // ============================================================================
    // DOWNLOAD MANAGER FUNCTIONS
    // ============================================================================

    /**
     * Enqueue a download using the persistent download manager.
     *
     * @param dbPath Path to SQLite database
     * @param accountJson Complete account JSON
     * @param asin Book ASIN
     * @param title Book title
     * @param outputDirectory Output directory (can be SAF URI)
     * @param quality Download quality
     * @return Promise resolving to Map with task_id
     */
    AsyncFunction("enqueueDownload") { dbPath: String, accountJson: String, asin: String, title: String, outputDirectory: String, quality: String ->
      try {
        DownloadService.enqueueBook(
          context = appContext.reactContext ?: throw Exception("Context not available"),
          dbPath = dbPath,
          accountJson = accountJson,
          asin = asin,
          title = title,
          outputDirectory = outputDirectory,
          quality = quality
        )

        mapOf(
          "success" to true,
          "data" to mapOf("message" to "Download enqueued")
        )
      } catch (e: Exception) {
        mapOf(
          "success" to false,
          "error" to "Enqueue download error: ${e.message}"
        )
      }
    }

    /**
     * Get download task status.
     *
     * @param dbPath Path to SQLite database
     * @param taskId Task ID
     * @return Map with task details
     */
    Function("getDownloadTask") { dbPath: String, taskId: String ->
      val params = JSONObject().apply {
        put("db_path", dbPath)
        put("task_id", taskId)
      }
      parseJsonResponse(nativeGetDownloadTask(params.toString()))
    }

    /**
     * List download tasks with optional filter.
     *
     * @param dbPath Path to SQLite database
     * @param filter Optional status filter ("queued", "downloading", "completed", "failed", etc.)
     * @return Map with list of tasks
     */
    Function("listDownloadTasks") { dbPath: String, filter: String? ->
      val params = JSONObject().apply {
        put("db_path", dbPath)
        filter?.let { put("filter", it) }
      }
      parseJsonResponse(nativeListDownloadTasks(params.toString()))
    }

    /**
     * Live per-ASIN stage progress (decrypting/validating/copying) + ETA for the
     * foreground download pipeline. Returns a JSON string:
     *   { "<asin>": { "stage": String, "percentage": Int, "eta_seconds": Long } }
     * Complements listDownloadTasks, whose DB rows lack stage percentage and ETA.
     */
    Function("getStageProgress") {
      StageProgressStore.snapshotJson()
    }

    /**
     * Pause a download.
     *
     * @param dbPath Path to SQLite database
     * @param taskId Task ID to pause
     * @return Map with success status
     */
    Function("pauseDownload") { dbPath: String, taskId: String ->
      val params = JSONObject().apply {
        put("db_path", dbPath)
        put("task_id", taskId)
      }
      parseJsonResponse(nativePauseDownload(params.toString()))
    }

    /**
     * Resume a paused download.
     *
     * @param dbPath Path to SQLite database
     * @param taskId Task ID to resume
     * @return Map with success status
     */
    Function("resumeDownload") { dbPath: String, taskId: String ->
      val params = JSONObject().apply {
        put("db_path", dbPath)
        put("task_id", taskId)
      }
      parseJsonResponse(nativeResumeDownload(params.toString()))
    }

    /**
     * Cancel a download.
     *
     * @param dbPath Path to SQLite database
     * @param taskId Task ID to cancel
     * @return Map with success status
     */
    Function("cancelDownload") { dbPath: String, taskId: String ->
      val params = JSONObject().apply {
        put("db_path", dbPath)
        put("task_id", taskId)
      }
      parseJsonResponse(nativeCancelDownload(params.toString()))
    }

    // ============================================================================
    // BACKGROUND TASK MANAGER FUNCTIONS (New System)
    // ============================================================================

    /**
     * Compatibility no-op.
     * Periodic background work is scheduled through WorkManager.
     */
    Function("startBackgroundService") {
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        expo.modules.rustbridge.tasks.BackgroundTaskService.start(context)
        mapOf("success" to true)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Stop the background task service.
     * This will disable all automatic features (token refresh, library sync, auto-download).
     */
    Function("stopBackgroundService") {
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        expo.modules.rustbridge.tasks.BackgroundTaskService.stopService(context)
        mapOf("success" to true)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Enqueue a download using the new background task system.
     *
     * @param asin Book ASIN
     * @param title Book title
     * @param author Optional book author
     * @param accountJson Complete account JSON
     * @param outputDirectory Output directory (SAF URI)
     * @param quality Download quality
     * @return Map with task_id
     */
    AsyncFunction("enqueueDownloadNew") { asin: String, title: String, author: String?, accountJson: String, outputDirectory: String, quality: String ->
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        val dbPath = AppPaths.databasePath(context)

        // Use DownloadService for downloads
        DownloadService.enqueueBook(
          context = context,
          dbPath = dbPath,
          accountJson = accountJson,
          asin = asin,
          title = title,
          outputDirectory = outputDirectory,
          quality = quality
        )
        mapOf("success" to true, "data" to mapOf("message" to "Download enqueued"))
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Retry conversion for a failed download that still has the cached .aax file.
     *
     * @param dbPath Database path
     * @param asin Book ASIN
     * @return Map with success status
     */
    AsyncFunction("retryConversion") { dbPath: String, asin: String ->
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")

        DownloadService.retryConversion(
          context = context,
          dbPath = dbPath,
          asin = asin
        )
        mapOf("success" to true, "data" to mapOf("message" to "Conversion retry started"))
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Start library sync using WorkManager.
     *
     * @param fullSync Whether to do a full sync (default: false)
     * @return Map with task_id
     */
    AsyncFunction("startLibrarySyncNew") { fullSync: Boolean ->
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        expo.modules.rustbridge.tasks.BackgroundTaskService.startLibrarySync(context, fullSync)
        mapOf("success" to true, "data" to mapOf("message" to "Library sync scheduled"))
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Enable automatic downloads after library sync.
     */
    Function("enableAutoDownload") {
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        val manager = expo.modules.rustbridge.tasks.BackgroundTaskManager.getInstance(context)
        manager.enableAutoDownload()
        mapOf("success" to true)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Disable automatic downloads.
     */
    Function("disableAutoDownload") {
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        val manager = expo.modules.rustbridge.tasks.BackgroundTaskManager.getInstance(context)
        manager.disableAutoDownload()
        mapOf("success" to true)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Save account to SQLite database (single source of truth).
     *
     * @param dbPath Database path
     * @param accountJson Account JSON string
     */
    AsyncFunction("saveAccount") { dbPath: String, accountJson: String ->
      try {
        val params = JSONObject().apply {
          put("db_path", dbPath)
          put("account_json", accountJson)
        }
        val result = nativeSaveAccount(params.toString())
        parseJsonResponse(result)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Get primary account from SQLite database.
     *
     * @param dbPath Database path
     * @return Account JSON or null if no account exists
     */
    AsyncFunction("getPrimaryAccount") { dbPath: String ->
      try {
        val params = JSONObject().apply {
          put("db_path", dbPath)
        }
        val result = nativeGetPrimaryAccount(params.toString())
        parseJsonResponse(result)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Get one account from SQLite database.
     */
    AsyncFunction("getAccount") { dbPath: String, accountId: String ->
      try {
        val params = JSONObject().apply {
          put("db_path", dbPath)
          put("account_id", accountId)
        }
        val result = nativeGetAccount(params.toString())
        parseJsonResponse(result)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Get all accounts from SQLite database.
     */
    AsyncFunction("getAllAccounts") { dbPath: String ->
      try {
        val params = JSONObject().apply {
          put("db_path", dbPath)
        }
        val result = nativeGetAllAccounts(params.toString())
        parseJsonResponse(result)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Delete account from SQLite database.
     *
     * @param dbPath Database path
     * @param accountId Account identifier
     */
    AsyncFunction("deleteAccount") { dbPath: String, accountId: String ->
      try {
        val params = JSONObject().apply {
          put("db_path", dbPath)
          put("account_id", accountId)
        }
        val result = nativeDeleteAccount(params.toString())
        parseJsonResponse(result)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Enable automatic library sync.
     *
     * @param intervalHours Sync interval in hours (default: 24)
     */
    Function("enableAutoSync") { intervalHours: Int? ->
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        val prefs = context.getSharedPreferences("library_sync_prefs", Context.MODE_PRIVATE)

        // Set interval if provided
        if (intervalHours != null && intervalHours > 0) {
          prefs.edit().putInt("sync_interval_hours", intervalHours).apply()
        }

        prefs.edit().putBoolean("auto_sync_enabled", true).apply()
        mapOf("success" to true, "data" to mapOf("intervalHours" to (intervalHours ?: 24)))
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Disable automatic library sync.
     */
    Function("disableAutoSync") {
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        val prefs = context.getSharedPreferences("library_sync_prefs", Context.MODE_PRIVATE)
        prefs.edit().putBoolean("auto_sync_enabled", false).apply()
        mapOf("success" to true)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Pause a task.
     *
     * @param taskId Task ID to pause
     */
    AsyncFunction("pauseTask") { taskId: String ->
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        val manager = expo.modules.rustbridge.tasks.BackgroundTaskManager.getInstance(context)
        val success = kotlinx.coroutines.runBlocking {
          manager.pauseTask(taskId)
        }
        mapOf("success" to success)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Resume a paused task.
     *
     * @param taskId Task ID to resume
     */
    AsyncFunction("resumeTask") { taskId: String ->
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        val manager = expo.modules.rustbridge.tasks.BackgroundTaskManager.getInstance(context)
        val success = kotlinx.coroutines.runBlocking {
          manager.resumeTask(taskId)
        }
        mapOf("success" to success)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Cancel a task.
     *
     * @param taskId Task ID to cancel
     */
    AsyncFunction("cancelTask") { taskId: String ->
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        val manager = expo.modules.rustbridge.tasks.BackgroundTaskManager.getInstance(context)
        val success = kotlinx.coroutines.runBlocking {
          manager.cancelTask(taskId)
        }
        mapOf("success" to success)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Get all active tasks.
     *
     * @return List of active tasks with their details
     */
    Function("getActiveTasks") {
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        val manager = expo.modules.rustbridge.tasks.BackgroundTaskManager.getInstance(context)
        val tasks = manager.getActiveTasks()

        val taskMaps = tasks.map { task ->
          mapOf(
            "id" to task.id,
            "type" to task.type.name,
            "priority" to task.priority.name,
            "status" to task.status.name,
            "metadata" to task.metadata,
            "createdAt" to task.createdAt.time,
            "startedAt" to task.startedAt?.time,
            "completedAt" to task.completedAt?.time,
            "error" to task.error
          )
        }

        mapOf("success" to true, "data" to taskMaps)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Get a specific task by ID.
     *
     * @param taskId Task ID
     * @return Task details or null if not found
     */
    Function("getTask") { taskId: String ->
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        val manager = expo.modules.rustbridge.tasks.BackgroundTaskManager.getInstance(context)
        val task = manager.getTask(taskId)

        if (task != null) {
          mapOf(
            "success" to true,
            "data" to mapOf(
              "id" to task.id,
              "type" to task.type.name,
              "priority" to task.priority.name,
              "status" to task.status.name,
              "metadata" to task.metadata,
              "createdAt" to task.createdAt.time,
              "startedAt" to task.startedAt?.time,
              "completedAt" to task.completedAt?.time,
              "error" to task.error
            )
          )
        } else {
          mapOf("success" to false, "error" to "Task not found")
        }
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Clear all tasks (for debugging/recovery from stuck states).
     */
    Function("clearAllTasks") {
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        val manager = expo.modules.rustbridge.tasks.BackgroundTaskManager.getInstance(context)
        manager.clearAllTasks()
        mapOf("success" to true)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Legacy compatibility status.
     * Background sync is now scheduled work, not a persistent foreground service.
     *
     * @return Map with isRunning boolean
     */
    Function("isBackgroundServiceRunning") {
      try {
        mapOf("success" to true, "data" to mapOf("isRunning" to false))
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Clear download state for all books.
     *
     * Resets download status but keeps all book metadata.
     *
     * @param dbPath Database path
     */
    AsyncFunction("clearDownloadState") { dbPath: String ->
      try {
        val params = JSONObject().apply {
          put("db_path", dbPath)
        }
        val result = nativeClearDownloadState(params.toString())
        parseJsonResponse(result)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Get the downloaded file path for a book by ASIN.
     *
     * @param dbPath Database path
     * @param asin Book ASIN (Audible product ID)
     * @return Map with file_path (null if not found)
     */
    AsyncFunction("getBookFilePath") { dbPath: String, asin: String ->
      try {
        val params = JSONObject().apply {
          put("db_path", dbPath)
          put("asin", asin)
        }
        val result = nativeGetBookFilePath(params.toString())
        parseJsonResponse(result)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Clear download state for a single book by ASIN.
     *
     * This marks a book as not downloaded, clearing its download status
     * and removing any download tasks to reset to default state.
     * Optionally deletes the downloaded file from disk.
     *
     * @param dbPath Database path
     * @param asin Book ASIN (Audible product ID)
     * @param deleteFile Whether to delete the downloaded file
     */
    AsyncFunction("clearBookDownloadState") { dbPath: String, asin: String, deleteFile: Boolean ->
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        var cleanupResult = DeleteCleanupResult(fileDeleted = false)
        var deleteError: String? = null
        var rustShouldDeleteFile = deleteFile

        if (deleteFile) {
          val filePath = getBookFilePathForDeletion(dbPath, asin)
          if (!filePath.isNullOrBlank()) {
            rustShouldDeleteFile = false
            try {
              cleanupResult = deleteDownloadedFile(context, filePath)
              if (!cleanupResult.fileDeleted) {
                deleteError = "File deletion returned false"
              }
            } catch (e: Exception) {
              deleteError = e.message ?: e.javaClass.simpleName
              android.util.Log.w("ExpoRustBridge", "Failed to delete downloaded file $filePath", e)
            }
          }
        }

        val params = JSONObject().apply {
          put("db_path", dbPath)
          put("asin", asin)
          put("delete_file", rustShouldDeleteFile)
        }
        val result = nativeClearBookDownloadState(params.toString())
        val parsed = parseJsonResponse(result)

        if (deleteFile && !rustShouldDeleteFile && parsed["success"] == true) {
          val data = mutableMapOf<String, Any?>()
          (parsed["data"] as? Map<*, *>)?.forEach { (key, value) ->
            if (key is String) {
              data[key] = value
            }
          }
          data["file_deleted"] = cleanupResult.fileDeleted
          data["deleted_path"] = cleanupResult.deletedPath
          data["cover_deleted"] = cleanupResult.coverDeleted
          data["book_folder_deleted"] = cleanupResult.bookFolderDeleted
          data["author_folder_deleted"] = cleanupResult.authorFolderDeleted
          cleanupResult.cleanupError?.let { data["cleanup_error"] = it }
          deleteError?.let { data["delete_error"] = it }

          mapOf("success" to true, "data" to data)
        } else {
          parsed
        }
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Set the file path for a book manually.
     *
     * Allows marking a book as downloaded by associating it with an existing
     * audio file. Creates a download task with status "completed".
     *
     * @param dbPath Database path
     * @param asin Audible product ID
     * @param title Book title
     * @param filePath Absolute path to the audio file
     */
    AsyncFunction("setBookFilePath") { dbPath: String, asin: String, title: String, filePath: String ->
      try {
        val params = JSONObject().apply {
          put("db_path", dbPath)
          put("asin", asin)
          put("title", title)
          put("file_path", filePath)
        }
        val result = nativeSetBookFilePath(params.toString())
        parseJsonResponse(result)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Create cover art file (EmbeddedCover.jpg) for a book.
     *
     * Downloads and saves the book's cover image as EmbeddedCover.jpg (500x500)
     * in the same directory as the audio file for Smart Audiobook Player compatibility.
     *
     * @param asin Audible product ID
     * @param coverUrl URL of the cover image
     * @param audioFilePath Path to the audio file (cover will be saved in same directory)
     */
    AsyncFunction("createCoverArtFile") { asin: String, coverUrl: String, audioFilePath: String ->
      var coverFile: File? = null
      var originalBitmap: android.graphics.Bitmap? = null
      var resizedBitmap: android.graphics.Bitmap? = null

      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")

        // Download cover image to cache
        val cacheDir = context.cacheDir
        val cacheCoverFile = File(cacheDir, "cover_$asin.jpg")
        coverFile = cacheCoverFile

        // Download the cover image
        val url = java.net.URL(coverUrl.replace(Regex("_SL\\d+_"), "_SL500_"))
        val connection = url.openConnection() as java.net.HttpURLConnection
        connection.connectTimeout = 10000
        connection.readTimeout = 10000
        connection.connect()

        if (connection.responseCode != 200) {
          throw Exception("Failed to download cover image: HTTP ${connection.responseCode}")
        }

        cacheCoverFile.outputStream().use { output ->
          connection.inputStream.use { input ->
            input.copyTo(output)
          }
        }

        // Load and resize cover image
        val original = android.graphics.BitmapFactory.decodeFile(cacheCoverFile.absolutePath)
          ?: throw Exception("Failed to decode cover image")
        originalBitmap = original

        val resized = android.graphics.Bitmap.createScaledBitmap(
          original,
          500,
          500,
          true
        )
        resizedBitmap = resized

        val coverPath = writeEmbeddedCover(context, audioFilePath, resized)

        mapOf(
          "success" to true,
          "data" to mapOf(
            "coverPath" to coverPath,
            "message" to "Cover art created successfully"
          )
        )
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      } finally {
        if (resizedBitmap != originalBitmap) {
          resizedBitmap?.recycle()
        }
        originalBitmap?.recycle()
        coverFile?.delete()
      }
    }

    /**
     * Clear all library data (for testing).
     *
     * @param dbPath Database path
     */
    AsyncFunction("clearLibrary") { dbPath: String ->
      try {
        val params = JSONObject().apply {
          put("db_path", dbPath)
        }
        val result = nativeClearLibrary(params.toString())
        parseJsonResponse(result)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    // ========================================================================
    // LibriVox Functions
    // ========================================================================

    AsyncFunction("insertLibrivoxBook") { dbPath: String, bookJson: String ->
      try {
        val bookData = JSONObject(bookJson)
        val params = JSONObject().apply {
          put("db_path", dbPath)
          put("librivox_id", bookData.getString("librivox_id"))
          put("title", bookData.getString("title"))
          put("authors", bookData.getJSONArray("authors"))
          put("narrators", bookData.optJSONArray("narrators") ?: org.json.JSONArray())
          put("description", bookData.optString("description", ""))
          put("length_in_minutes", bookData.optInt("length_in_minutes", 0))
          put("language", bookData.optString("language", "en"))
          if (bookData.has("cover_url")) put("cover_url", bookData.getString("cover_url"))
        }
        val result = nativeInsertLibrivoxBook(params.toString())
        parseJsonResponse(result)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    AsyncFunction("downloadLibrivoxFile") { librivoxId: String, title: String, author: String?, downloadUrl: String, outputDirectory: String ->
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")

        DownloadService.enqueueLibrivoxBook(
          context = context,
          librivoxId = librivoxId,
          title = title,
          author = author ?: "",
          downloadUrl = downloadUrl,
          outputDirectory = outputDirectory
        )

        mapOf(
          "success" to true,
          "data" to mapOf("message" to "LibriVox download enqueued")
        )
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
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

    // ============================================================================
    // BACKGROUND WORKER SCHEDULING (WorkManager)
    // ============================================================================

    /**
     * Schedule periodic token refresh worker (12 hour interval recommended).
     *
     * @param intervalHours How often to check for token expiry
     * @return Map with success flag
     */
    Function("scheduleTokenRefresh") { intervalHours: Int ->
      try {
        WorkerScheduler.scheduleTokenRefresh(appContext.reactContext!!, intervalHours.toLong())
        mapOf("success" to true)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Schedule periodic library sync worker.
     *
     * @param intervalHours How often to sync library (recommended: 24 hours)
     * @param wifiOnly If true, only sync on unmetered (WiFi) connections
     * @return Map with success flag
     */
    Function("scheduleLibrarySync") { intervalHours: Int, wifiOnly: Boolean ->
      try {
        WorkerScheduler.scheduleLibrarySync(appContext.reactContext!!, intervalHours.toLong(), wifiOnly)
        mapOf("success" to true)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Cancel token refresh worker.
     *
     * @return Map with success flag
     */
    Function("cancelTokenRefresh") {
      try {
        WorkerScheduler.cancelTokenRefresh(appContext.reactContext!!)
        mapOf("success" to true)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Cancel library sync worker.
     *
     * @return Map with success flag
     */
    Function("cancelLibrarySync") {
      try {
        WorkerScheduler.cancelLibrarySync(appContext.reactContext!!)
        mapOf("success" to true)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Cancel all background workers.
     *
     * @return Map with success flag
     */
    Function("cancelAllBackgroundTasks") {
      try {
        WorkerScheduler.cancelAllWork(appContext.reactContext!!)
        mapOf("success" to true)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Get status of token refresh worker.
     *
     * @return Map with worker state
     */
    Function("getTokenRefreshStatus") {
      try {
        val state = WorkerScheduler.getTokenRefreshStatus(appContext.reactContext!!)
        mapOf(
          "success" to true,
          "state" to (state?.toString() ?: "NOT_SCHEDULED")
        )
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Get status of library sync worker.
     *
     * @return Map with worker state
     */
    Function("getLibrarySyncStatus") {
      try {
        val state = WorkerScheduler.getLibrarySyncStatus(appContext.reactContext!!)
        mapOf(
          "success" to true,
          "state" to (state?.toString() ?: "NOT_SCHEDULED")
        )
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Set file naming pattern preference.
     *
     * @param pattern The naming pattern: "flat_file", "author_book_folder", or "author_series_book"
     * @return Map with success status
     */
    Function("setNamingPattern") { pattern: String ->
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        val prefs = context.getSharedPreferences("app_settings", Context.MODE_PRIVATE)
        prefs.edit().putString("naming_pattern", pattern).apply()
        mapOf("success" to true)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    /**
     * Get file naming pattern preference.
     *
     * @return Map with pattern value
     */
    Function("getNamingPattern") {
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        val prefs = context.getSharedPreferences("app_settings", Context.MODE_PRIVATE)
        val pattern = prefs.getString("naming_pattern", "author_series_book") ?: "author_series_book"
        mapOf("success" to true, "data" to mapOf("pattern" to pattern))
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    Function("setPodcastNamingPattern") { pattern: String ->
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        val prefs = context.getSharedPreferences("app_settings", Context.MODE_PRIVATE)
        prefs.edit().putString("podcast_naming_pattern", pattern).apply()
        mapOf("success" to true)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    Function("getPodcastNamingPattern") {
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        val prefs = context.getSharedPreferences("app_settings", Context.MODE_PRIVATE)
        val pattern = prefs.getString("podcast_naming_pattern", "podcast_episode_folder")
          ?: "podcast_episode_folder"
        mapOf("success" to true, "data" to mapOf("pattern" to pattern))
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    Function("setValidationLevel") { level: String ->
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        val prefs = context.getSharedPreferences("app_settings", Context.MODE_PRIVATE)
        prefs.edit().putString("validation_level", level).apply()
        mapOf("success" to true)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    Function("getValidationLevel") {
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        val prefs = context.getSharedPreferences("app_settings", Context.MODE_PRIVATE)
        val level = prefs.getString("validation_level", "full") ?: "full"
        mapOf("success" to true, "data" to mapOf("level" to level))
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    Function("setSmartPlayerCover") { enabled: Boolean ->
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        val prefs = context.getSharedPreferences("app_settings", Context.MODE_PRIVATE)
        prefs.edit().putString("smart_player_cover_enabled", enabled.toString()).apply()
        mapOf("success" to true)
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

    Function("getSmartPlayerCover") {
      try {
        val context = appContext.reactContext ?: throw Exception("Context not available")
        val prefs = context.getSharedPreferences("app_settings", Context.MODE_PRIVATE)
        val enabled = prefs.getString("smart_player_cover_enabled", "false") == "true"
        mapOf("success" to true, "data" to mapOf("enabled" to enabled))
      } catch (e: Exception) {
        mapOf("success" to false, "error" to e.message)
      }
    }

  }

  // ============================================================================
  // JSON PARSING HELPERS
  // ============================================================================

  /**
   * Escape metadata value for FFmpeg command line.
   * Wraps in double quotes and escapes special characters.
   */
  private fun escapeMetadata(value: String): String {
    val escaped = value
      .replace("\\", "\\\\")  // Escape backslashes
      .replace("\"", "\\\"")  // Escape double quotes
    return "\"$escaped\""  // Wrap in double quotes
  }

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

  private data class LibraryImageEntry(
    val type: String,
    val title: String = "",
    val subtitle: String = "",
    val authors: String = "",
    val series: String = "",
    val length: String = "",
    val coverUrl: String = ""
  )

  private fun createLibraryExportImageFile(entriesJson: String, outputUri: String) {
    val entries = parseLibraryImageEntries(entriesJson)
    val bookCount = entries.count { it.type == "book" }
    val maxHeight = 30_000
    var columns = 1
    var width = 1_200
    var height = measureLibraryImageHeight(entries, columns)

    while (height > maxHeight && columns < 10) {
      columns += 1
      width = max(1_200, 96 + columns * 220 + (columns - 1) * 12)
      height = measureLibraryImageHeight(entries, columns)
    }

    if (height > maxHeight) {
      throw Exception("Library is too large for a single PNG export ($bookCount audiobooks)")
    }

    val bitmap = Bitmap.createBitmap(width, max(1, height), Bitmap.Config.RGB_565)
    val canvas = Canvas(bitmap)
    canvas.drawColor(Color.rgb(236, 239, 244))

    val paints = LibraryImagePaints()
    var y = 36f
    val pendingBooks = mutableListOf<LibraryImageEntry>()

    fun flushBooks() {
      if (pendingBooks.isEmpty()) return
      val gap = 12f
      val padding = 48f
      val cardWidth = (width - padding * 2 - gap * (columns - 1)) / columns
      val cardHeight = if (columns == 1) 150f else 172f

      pendingBooks.chunked(columns).forEach { row ->
        row.forEachIndexed { column, entry ->
          val x = padding + column * (cardWidth + gap)
          drawLibraryImageBook(canvas, entry, RectF(x, y, x + cardWidth, y + cardHeight), columns, paints)
        }
        y += cardHeight + gap
      }

      pendingBooks.clear()
    }

    entries.forEach { entry ->
      when (entry.type) {
        "book" -> pendingBooks.add(entry)
        "header" -> {
          flushBooks()
          y = drawLibraryImageHeader(canvas, entry, y, width.toFloat(), paints)
        }
        "group" -> {
          flushBooks()
          y = drawLibraryImageGroup(canvas, entry, y, width.toFloat(), paints)
        }
      }
    }
    flushBooks()

    val uri = Uri.parse(outputUri)
    if (uri.scheme != "file") {
      bitmap.recycle()
      throw Exception("PNG output must be a file URI")
    }

    val outputPath = Uri.decode(uri.path ?: throw Exception("Missing output path"))
    val outputFile = File(outputPath)
    outputFile.parentFile?.mkdirs()

    FileOutputStream(outputFile).use { stream ->
      if (!bitmap.compress(Bitmap.CompressFormat.PNG, 100, stream)) {
        throw Exception("Failed to encode PNG")
      }
    }
    bitmap.recycle()
  }

  private fun parseLibraryImageEntries(entriesJson: String): List<LibraryImageEntry> {
    val jsonArray = JSONArray(entriesJson)
    return (0 until jsonArray.length()).map { index ->
      val item = jsonArray.getJSONObject(index)
      LibraryImageEntry(
        type = item.optString("type"),
        title = item.optString("title"),
        subtitle = item.optString("subtitle"),
        authors = item.optString("authors"),
        series = item.optString("series"),
        length = item.optString("length"),
        coverUrl = item.optString("cover_url")
      )
    }
  }

  private fun measureLibraryImageHeight(entries: List<LibraryImageEntry>, columns: Int): Int {
    var height = 72
    var pendingBooks = 0
    val cardHeight = if (columns == 1) 150 else 172
    val gap = 12

    fun flushBooks() {
      if (pendingBooks == 0) return
      val rows = ceil(pendingBooks.toDouble() / columns.toDouble()).toInt()
      height += rows * (cardHeight + gap)
      pendingBooks = 0
    }

    entries.forEach { entry ->
      when (entry.type) {
        "book" -> pendingBooks += 1
        "header" -> {
          flushBooks()
          height += 104
        }
        "group" -> {
          flushBooks()
          height += 58
        }
      }
    }

    flushBooks()
    return height + 36
  }

  private class LibraryImagePaints {
    val title = Paint(Paint.ANTI_ALIAS_FLAG).apply {
      color = Color.rgb(46, 52, 64)
      textSize = 32f
      typeface = Typeface.DEFAULT_BOLD
    }
    val subtitle = Paint(Paint.ANTI_ALIAS_FLAG).apply {
      color = Color.rgb(76, 86, 106)
      textSize = 20f
    }
    val group = Paint(Paint.ANTI_ALIAS_FLAG).apply {
      color = Color.rgb(46, 52, 64)
      textSize = 21f
      typeface = Typeface.DEFAULT_BOLD
    }
    val bookTitle = Paint(Paint.ANTI_ALIAS_FLAG).apply {
      color = Color.rgb(46, 52, 64)
      textSize = 18f
      typeface = Typeface.DEFAULT_BOLD
    }
    val metadata = Paint(Paint.ANTI_ALIAS_FLAG).apply {
      color = Color.rgb(76, 86, 106)
      textSize = 15f
    }
    val length = Paint(Paint.ANTI_ALIAS_FLAG).apply {
      color = Color.rgb(94, 129, 172)
      textSize = 15f
      typeface = Typeface.DEFAULT_BOLD
    }
    val card = Paint(Paint.ANTI_ALIAS_FLAG).apply {
      color = Color.WHITE
      style = Paint.Style.FILL
    }
    val border = Paint(Paint.ANTI_ALIAS_FLAG).apply {
      color = Color.rgb(216, 222, 233)
      style = Paint.Style.STROKE
      strokeWidth = 1f
    }
    val accent = Paint(Paint.ANTI_ALIAS_FLAG).apply {
      color = Color.rgb(136, 192, 208)
      style = Paint.Style.FILL
    }
    val coverPlaceholder = Paint(Paint.ANTI_ALIAS_FLAG).apply {
      color = Color.rgb(216, 222, 233)
      style = Paint.Style.FILL
    }
    val coverText = Paint(Paint.ANTI_ALIAS_FLAG).apply {
      color = Color.rgb(76, 86, 106)
      textSize = 14f
      textAlign = Paint.Align.CENTER
      typeface = Typeface.DEFAULT_BOLD
    }
  }

  private fun drawLibraryImageHeader(
    canvas: Canvas,
    entry: LibraryImageEntry,
    y: Float,
    width: Float,
    paints: LibraryImagePaints
  ): Float {
    canvas.drawText(entry.title, 48f, y + 36f, paints.title)
    canvas.drawText(entry.subtitle, 48f, y + 68f, paints.subtitle)
    canvas.drawRect(48f, y + 92f, width - 48f, y + 96f, paints.accent)
    return y + 116f
  }

  private fun drawLibraryImageGroup(
    canvas: Canvas,
    entry: LibraryImageEntry,
    y: Float,
    width: Float,
    paints: LibraryImagePaints
  ): Float {
    val rect = RectF(48f, y, width - 48f, y + 42f)
    canvas.drawRoundRect(rect, 8f, 8f, paints.accent)
    canvas.drawText(entry.title, 64f, y + 28f, paints.group)
    return y + 58f
  }

  private fun drawLibraryImageBook(
    canvas: Canvas,
    entry: LibraryImageEntry,
    rect: RectF,
    columns: Int,
    paints: LibraryImagePaints
  ) {
    canvas.drawRoundRect(rect, 8f, 8f, paints.card)
    canvas.drawRoundRect(rect, 8f, 8f, paints.border)

    val innerPadding = if (columns == 1) 16f else 12f
    val coverSize = if (columns == 1) 112f else min(76f, rect.width() * 0.34f)
    val coverRect = RectF(
      rect.left + innerPadding,
      rect.top + innerPadding,
      rect.left + innerPadding + coverSize,
      rect.top + innerPadding + coverSize
    )

    val coverBitmap = loadCoverBitmap(entry.coverUrl, coverSize.toInt())
    if (coverBitmap != null) {
      canvas.drawBitmap(coverBitmap, null, coverRect, null)
      coverBitmap.recycle()
    } else {
      canvas.drawRoundRect(coverRect, 6f, 6f, paints.coverPlaceholder)
      canvas.drawText("AUDIO", coverRect.centerX(), coverRect.centerY() + 5f, paints.coverText)
    }

    val textLeft = coverRect.right + innerPadding
    val textWidth = rect.right - textLeft - innerPadding
    var textY = rect.top + innerPadding + 20f
    val titleLines = if (columns == 1) 2 else 3

    textY = drawWrappedText(canvas, entry.title, textLeft, textY, textWidth, paints.bookTitle, titleLines, 22f)
    textY = drawWrappedText(canvas, entry.authors, textLeft, textY + 4f, textWidth, paints.metadata, 1, 18f)
    if (entry.series.isNotBlank()) {
      textY = drawWrappedText(canvas, entry.series, textLeft, textY + 2f, textWidth, paints.metadata, 1, 18f)
    }
    canvas.drawText(entry.length, textLeft, textY + 20f, paints.length)
  }

  private fun drawWrappedText(
    canvas: Canvas,
    text: String,
    x: Float,
    y: Float,
    maxWidth: Float,
    paint: Paint,
    maxLines: Int,
    lineHeight: Float
  ): Float {
    if (text.isBlank() || maxWidth <= 0f) return y

    var remaining = text.trim()
    var currentY = y
    var lines = 0

    while (remaining.isNotEmpty() && lines < maxLines) {
      var count = paint.breakText(remaining, true, maxWidth, null)
      if (count <= 0) count = 1

      var end = count
      if (count < remaining.length) {
        val lastSpace = remaining.substring(0, count).lastIndexOf(' ')
        if (lastSpace > 0) end = lastSpace
      }

      var line = remaining.substring(0, end).trim()
      remaining = remaining.drop(end).trimStart()

      if (remaining.isNotEmpty() && lines == maxLines - 1) {
        while (paint.measureText("$line...") > maxWidth && line.length > 1) {
          line = line.dropLast(1)
        }
        line = "$line..."
        remaining = ""
      }

      canvas.drawText(line, x, currentY, paint)
      currentY += lineHeight
      lines += 1
    }

    return currentY
  }

  private fun loadCoverBitmap(urlString: String, size: Int): Bitmap? {
    if (urlString.isBlank() || size <= 0) return null

    return try {
      val connection = URL(urlString).openConnection()
      connection.connectTimeout = 2_500
      connection.readTimeout = 4_000
      connection.getInputStream().use { stream ->
        val original = BitmapFactory.decodeStream(stream) ?: return null
        val scaled = Bitmap.createScaledBitmap(original, size, size, true)
        if (scaled != original) original.recycle()
        scaled
      }
    } catch (_: Exception) {
      null
    }
  }

  private data class DeleteCleanupResult(
    val fileDeleted: Boolean,
    val deletedPath: String? = null,
    val coverDeleted: Boolean = false,
    val bookFolderDeleted: Boolean = false,
    val authorFolderDeleted: Boolean = false,
    val cleanupError: String? = null
  )

  private data class TreeDocumentContext(
    val treeUri: Uri,
    val treeDocumentId: String,
    val documentId: String,
    val parentDocumentId: String?
  )

  private data class DocumentChild(
    val documentId: String,
    val displayName: String,
    val mimeType: String?
  )

  private fun getBookFilePathForDeletion(dbPath: String, asin: String): String? {
    val params = JSONObject().apply {
      put("db_path", dbPath)
      put("asin", asin)
    }
    val parsed = parseJsonResponse(nativeGetBookFilePath(params.toString()))
    if (parsed["success"] != true) {
      return null
    }

    val data = parsed["data"] as? Map<*, *> ?: return null
    return data["file_path"] as? String
  }

  private fun deleteDownloadedFile(context: Context, filePath: String): DeleteCleanupResult {
    return if (filePath.startsWith("content://")) {
      deleteContentDocument(context, Uri.parse(filePath), filePath)
    } else {
      deleteFilePath(filePath)
    }
  }

  private fun deleteFilePath(filePath: String): DeleteCleanupResult {
    val file = File(filePath.removePrefix("file://"))
    val fileDeleted = file.exists() && file.delete()

    return DeleteCleanupResult(
      fileDeleted = fileDeleted,
      deletedPath = if (fileDeleted) filePath else null
    )
  }

  private fun deleteContentDocument(context: Context, uri: Uri, originalPath: String): DeleteCleanupResult {
    val resolver = context.contentResolver
    val documentId = try {
      DocumentsContract.getDocumentId(uri)
    } catch (_: Exception) {
      null
    }
    val treeContext = documentId?.let { findTreeDocumentContext(context, it) }
    var fileDeleted = false

    try {
      fileDeleted = DocumentFile.fromSingleUri(context, uri)?.delete() == true
    } catch (e: Exception) {
      android.util.Log.d("ExpoRustBridge", "Single document delete failed for $uri: ${e.message}")
    }

    try {
      if (!fileDeleted && DocumentsContract.isDocumentUri(context, uri) && DocumentsContract.deleteDocument(resolver, uri)) {
        fileDeleted = true
      }
    } catch (e: Exception) {
      android.util.Log.d("ExpoRustBridge", "Direct document delete failed for $uri: ${e.message}")
    }

    if (!fileDeleted && treeContext != null) {
      try {
        fileDeleted = deleteDocumentById(context, treeContext.treeUri, treeContext.documentId)
      } catch (e: Exception) {
        android.util.Log.d("ExpoRustBridge", "Tree document delete failed for $uri: ${e.message}")
      }
    }

    if (!fileDeleted && treeContext != null) {
      try {
        fileDeleted = deleteDocumentByTreeWalk(context, treeContext)
      } catch (e: Exception) {
        android.util.Log.d("ExpoRustBridge", "Tree walk delete failed for $uri: ${e.message}")
      }
    }

    if (!fileDeleted) {
      fileDeleted = try {
        resolver.delete(uri, null, null) > 0
      } catch (e: Exception) {
        android.util.Log.d("ExpoRustBridge", "Content resolver delete failed for $uri: ${e.message}")
        false
      }
    }

    if (!fileDeleted) {
      return DeleteCleanupResult(fileDeleted = false)
    }

    var coverDeleted = false
    var bookFolderDeleted = false
    var authorFolderDeleted = false
    var cleanupError: String? = null

    if (treeContext?.parentDocumentId != null) {
      try {
        val cleanup = cleanupDeletedContentDocument(context, treeContext)
        coverDeleted = cleanup.coverDeleted
        bookFolderDeleted = cleanup.bookFolderDeleted
        authorFolderDeleted = cleanup.authorFolderDeleted
      } catch (e: Exception) {
        cleanupError = e.message ?: e.javaClass.simpleName
        android.util.Log.w("ExpoRustBridge", "Downloaded-file cleanup failed for $uri", e)
      }
    }

    return DeleteCleanupResult(
      fileDeleted = true,
      deletedPath = originalPath,
      coverDeleted = coverDeleted,
      bookFolderDeleted = bookFolderDeleted,
      authorFolderDeleted = authorFolderDeleted,
      cleanupError = cleanupError
    )
  }

  private fun cleanupDeletedContentDocument(
    context: Context,
    treeContext: TreeDocumentContext
  ): DeleteCleanupResult {
    val parentId = treeContext.parentDocumentId ?: return DeleteCleanupResult(fileDeleted = true)
    var coverDeleted = false
    var bookFolderDeleted = false
    var authorFolderDeleted = false

    val siblingsAfterFileDelete = listDocumentChildren(context, treeContext.treeUri, parentId)
    val hasOtherAudioFiles = siblingsAfterFileDelete.any { child ->
      child.documentId != treeContext.documentId && isAudioFile(child.displayName, child.mimeType)
    }

    if (!hasOtherAudioFiles) {
      val cover = siblingsAfterFileDelete.firstOrNull { it.displayName == "EmbeddedCover.jpg" }
      if (cover != null) {
        coverDeleted = deleteDocumentById(context, treeContext.treeUri, cover.documentId)
      }
    }

    if (parentId != treeContext.treeDocumentId &&
        isDocumentDirectoryEmpty(context, treeContext.treeUri, parentId)) {
      bookFolderDeleted = deleteDocumentById(context, treeContext.treeUri, parentId)

      if (bookFolderDeleted) {
        val authorDocumentId = getParentDocumentId(parentId)
        if (!authorDocumentId.isNullOrBlank() &&
            authorDocumentId != treeContext.treeDocumentId &&
            isDocumentDirectoryEmpty(context, treeContext.treeUri, authorDocumentId)) {
          authorFolderDeleted = deleteDocumentById(context, treeContext.treeUri, authorDocumentId)
        }
      }
    }

    return DeleteCleanupResult(
      fileDeleted = true,
      coverDeleted = coverDeleted,
      bookFolderDeleted = bookFolderDeleted,
      authorFolderDeleted = authorFolderDeleted
    )
  }

  private fun findTreeDocumentContext(context: Context, documentId: String): TreeDocumentContext? {
    return context.contentResolver.persistedUriPermissions
      .filter { it.isWritePermission }
      .mapNotNull { permission ->
        val treeDocumentId = try {
          DocumentsContract.getTreeDocumentId(permission.uri)
        } catch (_: Exception) {
          null
        }

        if (!treeDocumentId.isNullOrBlank() && isDocumentWithinTree(documentId, treeDocumentId)) {
          TreeDocumentContext(
            treeUri = permission.uri,
            treeDocumentId = treeDocumentId,
            documentId = documentId,
            parentDocumentId = getParentDocumentId(documentId)
          )
        } else {
          null
        }
      }
      .maxByOrNull { it.treeDocumentId.length }
  }

  private fun isDocumentWithinTree(documentId: String, treeDocumentId: String): Boolean {
    return documentId == treeDocumentId ||
      documentId.startsWith("$treeDocumentId/") ||
      (treeDocumentId.endsWith(":") && documentId.startsWith(treeDocumentId))
  }

  private fun getParentDocumentId(documentId: String): String? {
    val index = documentId.lastIndexOf('/')
    return if (index > 0) documentId.substring(0, index) else null
  }

  private fun listDocumentChildren(
    context: Context,
    treeUri: Uri,
    parentDocumentId: String
  ): List<DocumentChild> {
    val resolver = context.contentResolver
    val childrenUri = DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, parentDocumentId)
    val projection = arrayOf(
      DocumentsContract.Document.COLUMN_DOCUMENT_ID,
      DocumentsContract.Document.COLUMN_DISPLAY_NAME,
      DocumentsContract.Document.COLUMN_MIME_TYPE
    )
    val children = mutableListOf<DocumentChild>()

    resolver.query(childrenUri, projection, null, null, null)?.use { cursor ->
      val idColumn = cursor.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_DOCUMENT_ID)
      val nameColumn = cursor.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_DISPLAY_NAME)
      val mimeColumn = cursor.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_MIME_TYPE)

      while (cursor.moveToNext()) {
        children.add(
          DocumentChild(
            documentId = cursor.getString(idColumn),
            displayName = cursor.getString(nameColumn),
            mimeType = cursor.getString(mimeColumn)
          )
        )
      }
    }

    return children
  }

  private fun isDocumentDirectoryEmpty(context: Context, treeUri: Uri, documentId: String): Boolean {
    return listDocumentChildren(context, treeUri, documentId).isEmpty()
  }

  private fun deleteDocumentById(context: Context, treeUri: Uri, documentId: String): Boolean {
    val documentUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, documentId)
    return DocumentsContract.deleteDocument(context.contentResolver, documentUri)
  }

  private fun deleteDocumentByTreeWalk(context: Context, treeContext: TreeDocumentContext): Boolean {
    val root = DocumentFile.fromTreeUri(context, treeContext.treeUri) ?: return false
    val relativePath = relativeDocumentPath(treeContext.documentId, treeContext.treeDocumentId)
      ?: return false
    val parts = relativePath.split('/').filter { it.isNotBlank() }
    if (parts.isEmpty()) return false

    var current = root
    for (part in parts.dropLast(1)) {
      current = current.findFile(part)?.takeIf { it.isDirectory } ?: return false
    }

    return current.findFile(parts.last())?.delete() == true
  }

  private fun relativeDocumentPath(documentId: String, treeDocumentId: String): String? {
    return when {
      documentId == treeDocumentId -> null
      documentId.startsWith("$treeDocumentId/") -> documentId.removePrefix("$treeDocumentId/")
      treeDocumentId.endsWith(":") && documentId.startsWith(treeDocumentId) ->
        documentId.removePrefix(treeDocumentId).trimStart('/')
      else -> null
    }
  }

  private fun isAudioFile(displayName: String, mimeType: String?): Boolean {
    if (mimeType?.startsWith("audio/") == true) {
      return true
    }

    val lowerName = displayName.lowercase()
    return lowerName.endsWith(".m4b") ||
      lowerName.endsWith(".m4a") ||
      lowerName.endsWith(".mp4") ||
      lowerName.endsWith(".mp3") ||
      lowerName.endsWith(".aac") ||
      lowerName.endsWith(".flac") ||
      lowerName.endsWith(".ogg") ||
      lowerName.endsWith(".opus") ||
      lowerName.endsWith(".wav") ||
      lowerName.endsWith(".aax") ||
      lowerName.endsWith(".aaxc")
  }

  private fun writeEmbeddedCover(
    context: Context,
    audioFilePath: String,
    bitmap: android.graphics.Bitmap
  ): String {
    return if (audioFilePath.startsWith("content://")) {
      writeEmbeddedCoverToDocumentTree(context, Uri.parse(audioFilePath), bitmap)
    } else {
      val path = audioFilePath.removePrefix("file://")
      val targetDir = File(path).parentFile
        ?: throw Exception("Could not access parent directory")

      if (!targetDir.exists() || !targetDir.isDirectory) {
        throw Exception("Could not access parent directory")
      }

      if (!targetDir.canWrite()) {
        throw Exception("No write permission for parent directory")
      }

      val embeddedCover = File(targetDir, "EmbeddedCover.jpg")
      if (embeddedCover.exists() && !embeddedCover.delete()) {
        throw Exception("Failed to delete existing EmbeddedCover.jpg")
      }

      embeddedCover.outputStream().use { outputStream ->
        if (!bitmap.compress(android.graphics.Bitmap.CompressFormat.JPEG, 90, outputStream)) {
          throw Exception("Failed to write EmbeddedCover.jpg")
        }
      }

      embeddedCover.absolutePath
    }
  }

  private fun writeEmbeddedCoverToDocumentTree(
    context: Context,
    audioUri: Uri,
    bitmap: android.graphics.Bitmap
  ): String {
    val resolver = context.contentResolver
    val documentId = try {
      DocumentsContract.getDocumentId(audioUri)
    } catch (_: Exception) {
      null
    }

    val treeDocumentId = try {
      DocumentsContract.getTreeDocumentId(audioUri)
    } catch (_: Exception) {
      null
    }

    if (documentId.isNullOrBlank() || treeDocumentId.isNullOrBlank()) {
      throw Exception(
        "Cannot create EmbeddedCover.jpg next to this file because Android only granted access to the file, not its folder."
      )
    }

    val parentDocumentId = if (documentId.contains('/')) {
      documentId.substringBeforeLast('/')
    } else if (documentId != treeDocumentId) {
      treeDocumentId
    } else {
      ""
    }
    if (parentDocumentId.isBlank()) {
      throw Exception("Could not access parent directory")
    }

    deleteDocumentInDirectory(context, audioUri, parentDocumentId, "EmbeddedCover.jpg")

    val parentUri = DocumentsContract.buildDocumentUriUsingTree(audioUri, parentDocumentId)
    val embeddedCoverUri = DocumentsContract.createDocument(
      resolver,
      parentUri,
      "image/jpeg",
      "EmbeddedCover.jpg"
    ) ?: throw Exception("Failed to create EmbeddedCover.jpg")

    resolver.openOutputStream(embeddedCoverUri)?.use { outputStream ->
      if (!bitmap.compress(android.graphics.Bitmap.CompressFormat.JPEG, 90, outputStream)) {
        throw Exception("Failed to write EmbeddedCover.jpg")
      }
    } ?: throw Exception("Failed to open output stream for EmbeddedCover.jpg")

    return embeddedCoverUri.toString()
  }

  private fun deleteDocumentInDirectory(
    context: Context,
    treeUri: Uri,
    parentDocumentId: String,
    displayName: String
  ) {
    val resolver = context.contentResolver
    val childrenUri = DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, parentDocumentId)
    val projection = arrayOf(
      DocumentsContract.Document.COLUMN_DOCUMENT_ID,
      DocumentsContract.Document.COLUMN_DISPLAY_NAME
    )

    resolver.query(childrenUri, projection, null, null, null)?.use { cursor ->
      val idColumn = cursor.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_DOCUMENT_ID)
      val nameColumn = cursor.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_DISPLAY_NAME)

      while (cursor.moveToNext()) {
        if (cursor.getString(nameColumn) == displayName) {
          val childDocumentId = cursor.getString(idColumn)
          val childUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, childDocumentId)

          if (!DocumentsContract.deleteDocument(resolver, childUri)) {
            throw Exception("Failed to delete existing $displayName")
          }
        }
      }
    }
  }

  // ============================================================================
  // COMPANION OBJECT
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

    // All native methods accept a single JSON string parameter
    // Made static so DownloadService can access them
    @JvmStatic external fun nativeGenerateOAuthUrl(paramsJson: String): String
    @JvmStatic external fun nativeParseOAuthCallback(paramsJson: String): String
    @JvmStatic external fun nativeExchangeAuthCode(paramsJson: String): String
    @JvmStatic external fun nativeRefreshAccessToken(paramsJson: String): String
    @JvmStatic external fun nativeGetActivationBytes(paramsJson: String): String
    @JvmStatic external fun nativeInitDatabase(paramsJson: String): String
    @JvmStatic external fun nativeSyncLibrary(paramsJson: String): String
    @JvmStatic external fun nativeSyncLibraryPage(paramsJson: String): String
    @JvmStatic external fun nativeSyncPodcastEpisodes(paramsJson: String): String
    @JvmStatic external fun nativeGetBooks(paramsJson: String): String
    @JvmStatic external fun nativeGetBookByAsin(paramsJson: String): String
    @JvmStatic external fun nativeSearchBooks(paramsJson: String): String
    @JvmStatic external fun nativeGetBooksWithFilters(paramsJson: String): String
    @JvmStatic external fun nativeGetAllSeries(paramsJson: String): String
    @JvmStatic external fun nativeGetAllCategories(paramsJson: String): String
    @JvmStatic external fun nativeDownloadBook(paramsJson: String): String
    @JvmStatic external fun nativeDecryptAAX(paramsJson: String): String
    @JvmStatic external fun nativeValidateActivationBytes(paramsJson: String): String
    @JvmStatic external fun nativeGetSupportedLocales(paramsJson: String): String
    @JvmStatic external fun nativeBuildFilePath(paramsJson: String): String
    @JvmStatic external fun nativeGetCustomerInformation(paramsJson: String): String
    @JvmStatic external fun nativeLogFromRust(paramsJson: String): String
    @JvmStatic external fun nativeExportDatabase(paramsJson: String): String

    // License function (get license without downloading)
    @JvmStatic external fun nativeGetDownloadLicense(paramsJson: String): String

    // Download Manager functions
    @JvmStatic external fun nativeEnqueueDownload(paramsJson: String): String
    @JvmStatic external fun nativeGetDownloadTask(paramsJson: String): String
    @JvmStatic external fun nativeStartPendingDownloads(paramsJson: String): String
    @JvmStatic external fun nativeListDownloadTasks(paramsJson: String): String
    @JvmStatic external fun nativePauseDownload(paramsJson: String): String
    @JvmStatic external fun nativeResumeDownload(paramsJson: String): String
    @JvmStatic external fun nativeCancelDownload(paramsJson: String): String
    @JvmStatic external fun nativeUpdateDownloadTaskStatus(paramsJson: String): String
    @JvmStatic external fun nativeStoreConversionKeys(paramsJson: String): String

    // Account functions
    @JvmStatic external fun nativeSaveAccount(paramsJson: String): String
    @JvmStatic external fun nativeGetPrimaryAccount(paramsJson: String): String
    @JvmStatic external fun nativeGetAccount(paramsJson: String): String
    @JvmStatic external fun nativeGetAllAccounts(paramsJson: String): String
    @JvmStatic external fun nativeDeleteAccount(paramsJson: String): String

    // Testing functions
    @JvmStatic external fun nativeClearDownloadState(paramsJson: String): String
    @JvmStatic external fun nativeGetBookFilePath(paramsJson: String): String
    @JvmStatic external fun nativeClearBookDownloadState(paramsJson: String): String
    @JvmStatic external fun nativeSetBookFilePath(paramsJson: String): String
    @JvmStatic external fun nativeClearLibrary(paramsJson: String): String

    // LibriVox
    @JvmStatic external fun nativeInsertLibrivoxBook(paramsJson: String): String
  }
}
