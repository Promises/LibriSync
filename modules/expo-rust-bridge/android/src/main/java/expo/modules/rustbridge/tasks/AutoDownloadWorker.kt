package expo.modules.rustbridge.tasks

import android.content.Context
import android.content.SharedPreferences
import android.util.Log
import expo.modules.rustbridge.ExpoRustBridgeModule
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.filter
import org.json.JSONObject

/**
 * Worker for automatic downloads
 *
 * Features:
 * - Listens for LibrarySyncComplete events
 * - Finds books matching download criteria
 * - Enqueues downloads automatically
 * - Respects WiFi-only and storage limits
 * - Configurable download criteria (wishlist, series, etc.)
 */
class AutoDownloadWorker(
    private val context: Context,
    private val manager: BackgroundTaskManager
) {
    companion object {
        private const val TAG = "AutoDownloadWorker"
        private const val PREFS_NAME = "auto_download_prefs"
        private const val PREF_ENABLED = "enabled"
        private const val PREF_WIFI_ONLY = "wifi_only"
        private const val PREF_MAX_DOWNLOADS = "max_downloads"
        private const val PREF_CRITERIA = "criteria"
        private const val PAGE_SIZE = 200
    }

    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
    private val prefs: SharedPreferences = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
    private var eventListenerJob: Job? = null

    /**
     * Enable automatic downloads
     */
    fun enable() {
        Log.d(TAG, "Enabling auto-download")
        prefs.edit().putBoolean(PREF_ENABLED, true).apply()
        // Only arm the listener: auto-download runs after the NEXT library sync, not
        // immediately on enable (enabling should not suddenly start a batch of downloads).
        startEventListener()
    }

    /**
     * Re-arm the sync listener after a cold start if auto-download was left enabled in a
     * previous session. Does NOT scan immediately — only a sync should trigger downloads on
     * launch — this just ensures the listener exists (enable() is otherwise only called from
     * the UI, so a relaunch would leave nothing listening for LibrarySyncComplete).
     */
    fun resumeIfEnabled() {
        if (isEnabled()) {
            Log.d(TAG, "Auto-download was enabled previously; re-arming sync listener")
            startEventListener()
        }
    }

    /** Build and run one auto-download check (scan library, enqueue matches). */
    private suspend fun runCheck() {
        execute(
            Task(
                id = "auto_download_${System.currentTimeMillis()}",
                type = TaskType.AUTO_DOWNLOAD,
                priority = TaskPriority.MEDIUM,
                status = TaskStatus.PENDING
            )
        )
    }

    /**
     * Disable automatic downloads
     */
    fun disable() {
        Log.d(TAG, "Disabling auto-download")
        prefs.edit().putBoolean(PREF_ENABLED, false).apply()
        stopEventListener()
    }

    /**
     * Check if auto-download is enabled
     */
    fun isEnabled(): Boolean = prefs.getBoolean(PREF_ENABLED, false)

    /**
     * Execute an auto-download task
     */
    suspend fun execute(task: Task) = withContext(Dispatchers.IO) {
        try {
            Log.d(TAG, "Executing auto-download task")

            // Check if enabled
            if (!isEnabled()) {
                Log.d(TAG, "Auto-download is disabled, skipping")
                task.status = TaskStatus.CANCELLED
                task.completedAt = java.util.Date()
                manager.emitEvent(TaskEvent.TaskCancelled(task))
                manager.unregisterActiveTask(task.id)
                return@withContext
            }

            // Check WiFi if required
            val wifiOnly = prefs.getBoolean(PREF_WIFI_ONLY, true)
            if (wifiOnly && !manager.isWifiAvailable()) {
                Log.d(TAG, "WiFi required but not available, skipping")
                task.status = TaskStatus.CANCELLED
                task.completedAt = java.util.Date()
                manager.emitEvent(TaskEvent.TaskCancelled(task))
                manager.unregisterActiveTask(task.id)
                return@withContext
            }

            // Find books to download
            val booksToDownload = findBooksToDownload()

            if (booksToDownload.isEmpty()) {
                Log.d(TAG, "No books match auto-download criteria")
                task.status = TaskStatus.COMPLETED
                task.completedAt = java.util.Date()
                manager.emitEvent(TaskEvent.AutoDownloadComplete(task.id, 0))
                manager.emitEvent(TaskEvent.TaskCompleted(task))
                manager.unregisterActiveTask(task.id)
                return@withContext
            }

            Log.d(TAG, "Found ${booksToDownload.size} books to auto-download")
            manager.emitEvent(TaskEvent.AutoDownloadStarted(task.id, booksToDownload.size))

            // Enqueue downloads
            var downloadedCount = 0
            for (book in booksToDownload) {
                try {
                    enqueueDownload(book)
                    downloadedCount++
                } catch (e: Exception) {
                    Log.e(TAG, "Failed to enqueue download for ${book["asin"]}", e)
                }
            }

            // Mark as completed
            task.status = TaskStatus.COMPLETED
            task.completedAt = java.util.Date()
            manager.emitEvent(TaskEvent.AutoDownloadComplete(task.id, downloadedCount))
            manager.emitEvent(TaskEvent.TaskCompleted(task))
            manager.unregisterActiveTask(task.id)

            Log.d(TAG, "Auto-download complete: $downloadedCount downloads enqueued")

        } catch (e: Exception) {
            Log.e(TAG, "Auto-download failed", e)
            task.status = TaskStatus.FAILED
            task.error = e.message
            task.completedAt = java.util.Date()
            manager.emitEvent(TaskEvent.TaskFailed(task, e.message ?: "Auto-download failed"))
            manager.unregisterActiveTask(task.id)
        }
    }

    /**
     * Start listening for library sync completion
     */
    private fun startEventListener() {
        if (eventListenerJob?.isActive == true) {
            Log.w(TAG, "Event listener already running")
            return
        }

        Log.d(TAG, "Starting event listener for library sync completion")

        // The shared event flow replays buffered events to new collectors, so without this
        // cutoff, enabling auto-download after a sync already ran this session would replay
        // that old LibrarySyncComplete and start downloads immediately — exactly the
        // "downloads on enable" behavior this listener-only design avoids.
        val armedAt = System.currentTimeMillis()
        eventListenerJob = scope.launch {
            manager.eventFlow
                .filter { it is TaskEvent.LibrarySyncComplete && it.emittedAt >= armedAt }
                .collect { event ->
                    Log.d(TAG, "Library sync completed, triggering auto-download check")
                    runCheck()
                }
        }
    }

    /**
     * Stop event listener
     */
    private fun stopEventListener() {
        Log.d(TAG, "Stopping event listener")
        eventListenerJob?.cancel()
        eventListenerJob = null
    }

    /**
     * Find books that match auto-download criteria
     */
    private suspend fun findBooksToDownload(): List<Map<String, Any>> = withContext(Dispatchers.IO) {
        try {
            val dbPath = manager.getDbPath()
            val maxDownloads = prefs.getInt(PREF_MAX_DOWNLOADS, 10)
            val candidates = mutableListOf<Map<String, Any>>()
            var offset = 0

            // Page through the library exactly like ExistingDownloadScanner. The `file_path`
            // (the "already downloaded" signal) is only populated by the filtered query, and
            // books expose `audible_product_id`/`authors[]`, not `asin`/`author` — the plain
            // getBooks call and the old `download_status` field never carried this data.
            while (candidates.size < maxDownloads) {
                val params = JSONObject().apply {
                    put("db_path", dbPath)
                    put("offset", offset)
                    put("limit", PAGE_SIZE)
                    put("sort_field", "title")
                    put("sort_direction", "asc")
                }
                val resultObj = JSONObject(ExpoRustBridgeModule.nativeGetBooksWithFilters(params.toString()))
                if (!resultObj.optBoolean("success")) {
                    // Error, not warning: a failure on the first page means auto-download is
                    // effectively dead this round while the task still ends "completed".
                    Log.e(TAG, "Failed to get books (offset=$offset): ${resultObj.optString("error")}")
                    break
                }

                val data = resultObj.optJSONObject("data") ?: break
                val booksArray = data.optJSONArray("books") ?: break
                if (booksArray.length() == 0) break

                for (i in 0 until booksArray.length()) {
                    val book = booksArray.optJSONObject(i) ?: continue
                    // Skip books that can't be downloaded or are already downloaded (have a file).
                    if (!book.optBoolean("is_downloadable", false)) continue
                    val filePath = if (book.isNull("file_path")) "" else book.optString("file_path")
                    if (filePath.isNotBlank()) continue

                    val asin = book.optString("audible_product_id")
                    val title = book.optString("title")
                    if (asin.isBlank() || title.isBlank()) continue

                    candidates.add(
                        mapOf(
                            "asin" to asin,
                            "title" to title,
                            "author" to joinStrings(book.optJSONArray("authors"))
                        )
                    )
                    if (candidates.size >= maxDownloads) break
                }

                offset += booksArray.length()
            }

            candidates

        } catch (e: Exception) {
            Log.e(TAG, "Error finding books to download", e)
            emptyList()
        }
    }

    /** Join a JSON string array (e.g. authors) into a display string. */
    private fun joinStrings(arr: org.json.JSONArray?): String {
        if (arr == null) return ""
        return (0 until arr.length())
            .mapNotNull { arr.optString(it).takeIf { s -> s.isNotBlank() } }
            .joinToString(", ")
    }

    /**
     * Enqueue a download for a book
     */
    private suspend fun enqueueDownload(book: Map<String, Any>) = withContext(Dispatchers.IO) {
        try {
            val asin = book["asin"] as? String ?: throw Exception("No ASIN")
            val title = book["title"] as? String ?: throw Exception("No title")

            // Load account and output directory from preferences
            val accountJson = getAccountJson() ?: throw Exception("No account")
            val outputDir = getOutputDirectory() ?: throw Exception("No output directory")

            Log.d(TAG, "Enqueueing auto-download via the foreground download pipeline: $asin - $title")

            // One engine: route auto-downloads through the same DownloadService/DownloadOrchestrator
            // that manual downloads use, so they get identical stage controls, per-stage cancel,
            // cleanup, per-book notifications and ASIN de-duplication — no second pipeline to keep
            // in sync (the two-engine split is what caused the stuck tasks and cross-cancel bugs).
            expo.modules.rustbridge.DownloadService.enqueueBook(
                context,
                manager.getDbPath(),
                accountJson,
                asin,
                title,
                outputDir,
                "High"
            )

        } catch (e: Exception) {
            Log.e(TAG, "Failed to enqueue download", e)
            throw e
        }
    }

    /**
     * Get account JSON from SQLite database
     */
    private fun getAccountJson(): String? {
        return try {
            val getAccountParams = org.json.JSONObject().apply {
                put("db_path", manager.getDbPath())
            }
            val accountResultJson = ExpoRustBridgeModule.nativeGetPrimaryAccount(getAccountParams.toString())
            val accountResultObj = org.json.JSONObject(accountResultJson)

            if (!accountResultObj.getBoolean("success")) {
                android.util.Log.d(TAG, "Failed to get account from database")
                null
            } else {
                val accountJson = accountResultObj.getJSONObject("data").optString("account")
                if (accountJson.isNullOrEmpty() || accountJson == "null") null else accountJson
            }
        } catch (e: Exception) {
            android.util.Log.e(TAG, "Error getting account from database", e)
            null
        }
    }

    /**
     * Get output directory from preferences
     */
    private fun getOutputDirectory(): String? {
        // TODO: Get from user settings
        // For now, return a default path
        val defaultDir = context.getExternalFilesDir(null)?.absolutePath
        return defaultDir?.let { "file://$it/audiobooks" }
    }
}
