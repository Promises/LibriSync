package expo.modules.rustbridge

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.SharedPreferences
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.Build
import android.util.Log
import kotlinx.coroutines.*
import org.json.JSONObject
import java.io.File
import androidx.documentfile.provider.DocumentFile
import android.net.Uri

/**
 * Download Orchestrator - Manages the complete download → conversion pipeline
 *
 * Responsibilities:
 * - Manages download queue via Rust PersistentDownloadManager
 * - Monitors download completion and triggers conversions
 * - Manages WiFi-only mode (pauses downloads when WiFi lost)
 * - Coordinates with ConversionManager for decryption
 * - Handles final file copying to user's SAF directory
 * - Provides progress callbacks to UI
 */
class DownloadOrchestrator(
    private val context: Context,
    private val dbPath: String
) {
    companion object {
        private const val TAG = "DownloadOrchestrator"
        private const val PREFS_NAME = "download_orchestrator_prefs"
        private const val PREF_WIFI_ONLY = "wifi_only_mode"
        private const val PREF_MANUALLY_PAUSED = "manually_paused_asins"
    }

    private val conversionManager = ConversionManager(context)
    private val prefs: SharedPreferences = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())

    // Network monitoring
    private val connectivityManager = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    private var isWifiAvailable = false

    // Active download monitoring jobs
    private val monitoringJobs = mutableMapOf<String, Job>()

    // Callbacks
    private var progressCallback: ((String, String, Double, Long, Long) -> Unit)? = null // (asin, stage, percentage, bytesDownloaded, totalBytes)
    private var completionCallback: ((String, String, String) -> Unit)? = null // (asin, title, outputPath)
    private var errorCallback: ((String, String, String) -> Unit)? = null // (asin, title, error)

    init {
        setupNetworkMonitoring()
        setupConversionCallbacks()
        resumePendingTasks()
    }

    /**
     * Get WiFi-only mode setting
     */
    fun isWifiOnlyMode(): Boolean {
        return prefs.getBoolean(PREF_WIFI_ONLY, false)
    }

    /**
     * Set WiFi-only mode
     */
    fun setWifiOnlyMode(enabled: Boolean) {
        prefs.edit().putBoolean(PREF_WIFI_ONLY, enabled).apply()
        Log.d(TAG, "WiFi-only mode: $enabled")

        scope.launch {
            if (enabled && !isWifiAvailable) {
                // Pause all active downloads
                pauseAllActiveDownloads()
            } else if (!enabled || isWifiAvailable) {
                // Resume paused downloads
                resumeAllPausedDownloads()
            }
        }
    }

    /**
     * Enqueue a book for download and conversion
     */
    suspend fun enqueueBook(
        accountJson: String,
        asin: String,
        title: String,
        outputDirectory: String,
        quality: String = "High"
    ): String = withContext(Dispatchers.IO) {
        Log.d(TAG, "Enqueueing book: $asin - $title")

        try {
            // Step 1: Get download license
            val licenseParams = JSONObject().apply {
                put("accountJson", accountJson)
                put("asin", asin)
                put("quality", quality)
            }

            val licenseResult = ExpoRustBridgeModule.nativeGetDownloadLicense(licenseParams.toString())
            val parsedLicense = parseJsonResponse(licenseResult)

            if (parsedLicense["success"] != true) {
                throw Exception("License request failed: ${parsedLicense["error"]}")
            }

            val licenseData = parsedLicense["data"] as? Map<*, *> ?: throw Exception("No license data")
            val downloadUrl = licenseData["download_url"] as? String ?: throw Exception("No download URL")
            val totalBytes = (licenseData["total_bytes"] as? Number)?.toLong() ?: 0L
            val aaxcKey = licenseData["aaxc_key"] as? String ?: throw Exception("No AAXC key")
            val aaxcIv = licenseData["aaxc_iv"] as? String ?: throw Exception("No AAXC IV")
            @Suppress("UNCHECKED_CAST")
            val requestHeaders = licenseData["request_headers"] as? Map<String, String>
                ?: mapOf("User-Agent" to "Audible/671 CFNetwork/1240.0.4 Darwin/20.6.0")

            Log.d(TAG, "License obtained. Size: ${totalBytes / 1024 / 1024} MB")

            // Step 2: Prepare paths
            val cacheDir = context.cacheDir
            val audiobooksDir = File(cacheDir, "audiobooks")
            audiobooksDir.mkdirs()

            val encryptedPath = File(audiobooksDir, "$asin.aax").absolutePath
            val decryptedCachePath = File(audiobooksDir, "$asin.m4b").absolutePath

            // Step 3: Enqueue download in Rust manager
            val enqueueParams = JSONObject().apply {
                put("db_path", dbPath)
                put("asin", asin)
                put("title", title)
                put("download_url", downloadUrl)
                put("total_bytes", totalBytes)
                put("download_path", encryptedPath)
                put("output_path", decryptedCachePath)
                put("request_headers", JSONObject(requestHeaders))
            }

            val enqueueResult = ExpoRustBridgeModule.nativeEnqueueDownload(enqueueParams.toString())
            val parsedEnqueue = parseJsonResponse(enqueueResult)

            if (parsedEnqueue["success"] != true) {
                throw Exception("Failed to enqueue: ${parsedEnqueue["error"]}")
            }

            val enqueueData = parsedEnqueue["data"] as? Map<*, *>
            val taskId = enqueueData?.get("task_id") as? String ?: throw Exception("No task ID")

            Log.d(TAG, "Download enqueued: $taskId")

            // Step 4: Start monitoring this download
            startMonitoringDownload(
                taskId = taskId,
                asin = asin,
                title = title,
                encryptedPath = encryptedPath,
                decryptedCachePath = decryptedCachePath,
                outputDirectory = outputDirectory,
                aaxcKey = aaxcKey,
                aaxcIv = aaxcIv,
                totalBytes = totalBytes
            )

            taskId
        } catch (e: Exception) {
            Log.e(TAG, "Failed to enqueue book", e)
            errorCallback?.invoke(asin, title, e.message ?: "Unknown error")
            throw e
        }
    }

    /**
     * Start monitoring a download for completion
     */
    private fun startMonitoringDownload(
        taskId: String,
        asin: String,
        title: String,
        encryptedPath: String,
        decryptedCachePath: String,
        outputDirectory: String,
        aaxcKey: String,
        aaxcIv: String,
        totalBytes: Long
    ) {
        // Cancel any existing monitoring for this ASIN
        monitoringJobs[asin]?.cancel()

        // Send initial progress notification (0%)
        progressCallback?.invoke(asin, "downloading", 0.0, 0, totalBytes)

        val job = scope.launch {
            try {
                while (isActive) {
                    delay(2000) // Poll every 2 seconds

                    // Check download status
                    val statusParams = JSONObject().apply {
                        put("db_path", dbPath)
                        put("task_id", taskId)
                    }

                    val statusResult = ExpoRustBridgeModule.nativeGetDownloadTask(statusParams.toString())
                    val parsedStatus = parseJsonResponse(statusResult)

                    if (parsedStatus["success"] == true) {
                        val taskData = parsedStatus["data"] as? Map<*, *>
                        val status = taskData?.get("status") as? String
                        val bytesDownloaded = (taskData?.get("bytes_downloaded") as? Number)?.toLong() ?: 0L
                        val taskTotalBytes = (taskData?.get("total_bytes") as? Number)?.toLong() ?: totalBytes
                        val percentage = (bytesDownloaded.toDouble() / taskTotalBytes) * 100.0

                        Log.d(TAG, "Download $asin: $status ($percentage%)")

                        when (status) {
                            "downloading" -> {
                                // Send progress notification only while downloading
                                progressCallback?.invoke(asin, "downloading", percentage, bytesDownloaded, taskTotalBytes)
                            }
                            "paused" -> {
                                Log.d(TAG, "Download paused for $asin - will resume monitoring when unpaused")
                                // Continue monitoring but don't send progress notifications
                                // This allows detection of resume events
                            }
                            "completed" -> {
                                Log.d(TAG, "Download completed! Triggering conversion for $asin")

                                // Trigger conversion (cancellable via coroutine scope)
                                try {
                                    triggerConversion(
                                        asin, title, encryptedPath, decryptedCachePath,
                                        outputDirectory, aaxcKey, aaxcIv
                                    )
                                } catch (e: CancellationException) {
                                    Log.d(TAG, "Conversion cancelled for $asin")
                                    throw e // Re-throw to exit the monitoring loop
                                }

                                // Stop monitoring
                                break
                            }
                            "failed" -> {
                                val error = taskData?.get("error") as? String ?: "Unknown error"
                                Log.e(TAG, "Download failed for $asin: $error")
                                errorCallback?.invoke(asin, title, error)
                                break
                            }
                            "cancelled" -> {
                                Log.d(TAG, "Download cancelled for $asin")
                                break
                            }
                        }
                    } else {
                        Log.e(TAG, "Failed to check status: ${parsedStatus["error"]}")
                        break
                    }
                }
            } catch (e: Exception) {
                Log.e(TAG, "Error monitoring download $asin", e)
            } finally {
                monitoringJobs.remove(asin)
            }
        }

        monitoringJobs[asin] = job
    }

    /**
     * Trigger conversion after download completes
     */
    private suspend fun triggerConversion(
        asin: String,
        title: String,
        encryptedPath: String,
        decryptedCachePath: String,
        outputDirectory: String,
        aaxcKey: String,
        aaxcIv: String
    ) = withContext(Dispatchers.IO) {
        try {
            Log.d(TAG, "Starting conversion for $asin...")

            // Notify decrypting stage
            progressCallback?.invoke(asin, "decrypting", 0.0, 0, 0)

            // Fetch metadata from database
            val metadata = fetchBookMetadata(asin)

            // Download cover art if available
            var coverArtPath: String? = null
            if (metadata != null) {
                val coverUrl = metadata["picture_large"] as? String
                if (coverUrl != null && coverUrl.isNotEmpty()) {
                    try {
                        val coverFile = File.createTempFile("cover_", ".jpg")
                        val url = java.net.URL(coverUrl)
                        url.openStream().use { input ->
                            coverFile.outputStream().use { output ->
                                input.copyTo(output)
                            }
                        }
                        coverArtPath = coverFile.absolutePath
                        Log.d(TAG, "Downloaded cover art for $asin: $coverArtPath")
                    } catch (e: Exception) {
                        Log.w(TAG, "Failed to download cover art for $asin: ${e.message}")
                    }
                }
            }

            // Decrypt using FFmpeg-Kit with metadata and cover art
            val command = buildList {
                add("-y")
                add("-audible_key")
                add(aaxcKey)
                add("-audible_iv")
                add(aaxcIv)
                add("-i")
                add(encryptedPath)

                // Add cover art input if available
                if (coverArtPath != null) {
                    add("-i")
                    add(coverArtPath)
                }

                // Add metadata tags if available
                if (metadata != null) {
                    // Title
                    metadata["title"]?.let {
                        add("-metadata")
                        add("title=${escapeMetadata(it.toString())}")
                    }

                    // Subtitle (append to description/comment)
                    metadata["subtitle"]?.let { subtitle ->
                        val description = metadata["description"]?.toString() ?: ""
                        val fullDesc = if (description.isNotEmpty()) {
                            "$description\n\nSubtitle: $subtitle"
                        } else {
                            "Subtitle: $subtitle"
                        }
                        add("-metadata")
                        add("comment=${escapeMetadata(fullDesc)}")
                    } ?: metadata["description"]?.let {
                        add("-metadata")
                        add("comment=${escapeMetadata(it.toString())}")
                    }

                    // Authors (artist tag)
                    metadata["authors"]?.let {
                        add("-metadata")
                        add("artist=${escapeMetadata(it.toString())}")
                        add("-metadata")
                        add("album_artist=${escapeMetadata(it.toString())}")
                    }

                    // Narrators (composer tag - standard for audiobooks)
                    metadata["narrators"]?.let {
                        add("-metadata")
                        add("composer=${escapeMetadata(it.toString())}")
                    }

                    // Publisher
                    metadata["publisher"]?.let { publisher ->
                        add("-metadata")
                        add("publisher=${escapeMetadata(publisher.toString())}")

                        // Copyright (format: ©YEAR Publisher;(P)YEAR Publisher)
                        val year = metadata["date_published"]?.toString()?.take(4) ?: "2024"
                        val copyright = "©$year $publisher;(P)$year $publisher"
                        add("-metadata")
                        add("copyright=${escapeMetadata(copyright)}")
                    }

                    // Series information (album tag)
                    val seriesName = metadata["series_name"]?.toString()
                    val seriesSequence = metadata["series_sequence"]
                    if (seriesName != null) {
                        val albumTag = if (seriesSequence != null) {
                            "$seriesName, Book $seriesSequence"
                        } else {
                            seriesName
                        }
                        add("-metadata")
                        add("album=${escapeMetadata(albumTag)}")
                    }

                    // Release date (year tag)
                    metadata["date_published"]?.toString()?.let { dateStr ->
                        // Extract year from date (format: YYYY-MM-DD or YYYY)
                        val year = dateStr.take(4)
                        add("-metadata")
                        add("date=${escapeMetadata(year)}")
                    }

                    // Language
                    metadata["language"]?.let {
                        add("-metadata")
                        add("language=${escapeMetadata(it.toString())}")
                    }

                    // Audible ASIN (grouping tag - perfect for tracking IDs)
                    metadata["audible_asin"]?.let {
                        add("-metadata")
                        add("grouping=${escapeMetadata(it.toString())}")
                    }

                    // Genre (always Audiobook)
                    add("-metadata")
                    add("genre=Audiobook")
                }

                // Map streams explicitly (audio + optional cover art)
                add("-map")
                add("0:a")  // Audio from encrypted file

                if (coverArtPath != null) {
                    add("-map")
                    add("1")    // Cover art from image file
                    add("-disposition:v:0")
                    add("attached_pic")
                    add("-c:v")
                    add("mjpeg")  // Encode cover as MJPEG
                } else {
                    // Skip all video streams (no cover art)
                    add("-vn")
                }

                add("-c:a")
                add("copy")  // Copy audio without re-encoding
                add(decryptedCachePath)
            }.joinToString(" ")

            val session = com.arthenica.ffmpegkit.FFmpegKit.execute(command)

            // Cleanup cover art temp file
            coverArtPath?.let { File(it).delete() }

            if (!com.arthenica.ffmpegkit.ReturnCode.isSuccess(session.returnCode)) {
                throw Exception("FFmpeg failed: ${session.failStackTrace}")
            }

            Log.d(TAG, "Conversion complete for $asin (with metadata + cover art)")

            // Notify copying stage
            progressCallback?.invoke(asin, "copying", 0.0, 0, 0)

            // Copy to final destination
            copyToFinalDestination(asin, title, decryptedCachePath, outputDirectory)

            // Cleanup encrypted file
            File(encryptedPath).delete()

        } catch (e: Exception) {
            Log.e(TAG, "Conversion failed for $asin", e)
            errorCallback?.invoke(asin, title, e.message ?: "Conversion failed")
        }
    }

    /**
     * Copy decrypted file to user's chosen directory
     */
    private suspend fun copyToFinalDestination(
        asin: String,
        title: String,
        decryptedCachePath: String,
        outputDirectory: String
    ) = withContext(Dispatchers.IO) {
        val cachedFile = File(decryptedCachePath)
        var finalPath = decryptedCachePath

        if (outputDirectory.startsWith("content://")) {
            // SAF URI - use DocumentFile
            val treeUri = Uri.parse(outputDirectory)
            val docDir = DocumentFile.fromTreeUri(context, treeUri)
                ?: throw Exception("Invalid SAF URI")

            if (!docDir.canWrite()) {
                throw Exception("No write permission for SAF directory")
            }

            // Build proper file path using naming pattern
            val filePath = buildFilePathForBook(asin)
            Log.d(TAG, "Using file path: $filePath")

            // Split path into directories and filename
            val pathParts = filePath.split('/')
            val fileName = pathParts.last()
            val directories = pathParts.dropLast(1)

            // Navigate/create subdirectories
            var currentDir = docDir
            for (dirName in directories) {
                val existing = currentDir.findFile(dirName)
                currentDir = if (existing != null && existing.isDirectory) {
                    existing
                } else {
                    currentDir.createDirectory(dirName)
                        ?: throw Exception("Failed to create directory: $dirName")
                }
            }

            // Delete existing file
            currentDir.findFile(fileName)?.delete()

            // Create new file
            val outputFile = currentDir.createFile("audio/mp4", fileName)
                ?: currentDir.createFile("audio/x-m4b", fileName)
                ?: currentDir.createFile("audio/*", fileName)
                ?: throw Exception("Failed to create file in SAF directory")

            Log.d(TAG, "Copying to SAF: ${outputFile.uri}")

            // Copy
            context.contentResolver.openOutputStream(outputFile.uri)?.use { outputStream ->
                cachedFile.inputStream().use { inputStream ->
                    inputStream.copyTo(outputStream)
                }
            } ?: throw Exception("Failed to open output stream")

            finalPath = outputFile.uri.toString()

            // Delete cache file
            cachedFile.delete()
        }

        Log.d(TAG, "Complete! Final path: $finalPath")

        // Clear manual pause marker on completion
        clearManuallyPaused(asin)

        completionCallback?.invoke(asin, title, finalPath)
    }

    /**
     * Setup network monitoring for WiFi-only mode
     */
    private fun setupNetworkMonitoring() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            val networkRequest = NetworkRequest.Builder()
                .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
                .build()

            networkCallback = object : ConnectivityManager.NetworkCallback() {
                override fun onAvailable(network: Network) {
                    Log.d(TAG, "WiFi available")
                    isWifiAvailable = true

                    if (isWifiOnlyMode()) {
                        // Resume paused downloads
                        scope.launch {
                            resumeAllPausedDownloads()
                        }
                    }
                }

                override fun onLost(network: Network) {
                    Log.d(TAG, "WiFi lost")
                    isWifiAvailable = false

                    if (isWifiOnlyMode()) {
                        // Pause all active downloads
                        scope.launch {
                            pauseAllActiveDownloads()
                        }
                    }
                }
            }

            connectivityManager.registerNetworkCallback(networkRequest, networkCallback!!)

            // Check initial WiFi state
            val network = connectivityManager.activeNetwork
            val capabilities = connectivityManager.getNetworkCapabilities(network)
            isWifiAvailable = capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true
        }
    }

    /**
     * Setup conversion callbacks
     */
    private fun setupConversionCallbacks() {
        conversionManager.setProgressListener { task ->
            // Pass 0 for bytes since we don't track conversion progress in bytes
            progressCallback?.invoke(task.asin, "converting", task.progress.percentage, 0, 0)
        }

        conversionManager.setCompletionListener { task ->
            Log.d(TAG, "Conversion completed for ${task.asin}")
            // completionCallback will be called from triggerConversion after file copy
        }
    }

    /**
     * Pause all active downloads
     */
    private suspend fun pauseAllActiveDownloads() = withContext(Dispatchers.IO) {
        try {
            val listParams = JSONObject().apply {
                put("db_path", dbPath)
                put("filter", "downloading")
            }

            val listResult = ExpoRustBridgeModule.nativeListDownloadTasks(listParams.toString())
            val parsed = parseJsonResponse(listResult)

            if (parsed["success"] == true) {
                val data = parsed["data"] as? Map<*, *>
                @Suppress("UNCHECKED_CAST")
                val tasks = data?.get("tasks") as? List<Map<*, *>> ?: emptyList()

                tasks.forEach { task ->
                    val taskId = task["task_id"] as? String ?: return@forEach

                    val pauseParams = JSONObject().apply {
                        put("db_path", dbPath)
                        put("task_id", taskId)
                    }

                    ExpoRustBridgeModule.nativePauseDownload(pauseParams.toString())
                    Log.d(TAG, "Paused download: $taskId (WiFi lost)")
                }
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error pausing downloads", e)
        }
    }

    /**
     * Resume all paused downloads (except manually paused ones)
     */
    private suspend fun resumeAllPausedDownloads() = withContext(Dispatchers.IO) {
        try {
            val listParams = JSONObject().apply {
                put("db_path", dbPath)
                put("filter", "paused")
            }

            val listResult = ExpoRustBridgeModule.nativeListDownloadTasks(listParams.toString())
            val parsed = parseJsonResponse(listResult)

            if (parsed["success"] == true) {
                val data = parsed["data"] as? Map<*, *>
                @Suppress("UNCHECKED_CAST")
                val tasks = data?.get("tasks") as? List<Map<*, *>> ?: emptyList()

                // Get list of manually paused downloads
                val manuallyPaused = getManuallyPausedAsins()

                tasks.forEach { task ->
                    val asin = task["asin"] as? String ?: return@forEach
                    val taskId = task["task_id"] as? String ?: return@forEach

                    // Skip manually paused downloads
                    if (manuallyPaused.contains(asin)) {
                        Log.d(TAG, "Skipping auto-resume for manually paused download: $asin")
                        return@forEach
                    }

                    val resumeParams = JSONObject().apply {
                        put("db_path", dbPath)
                        put("task_id", taskId)
                    }

                    ExpoRustBridgeModule.nativeResumeDownload(resumeParams.toString())
                    Log.d(TAG, "Resumed download: $taskId (WiFi available)")
                }
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error resuming downloads", e)
        }
    }

    /**
     * Mark an ASIN as manually paused
     */
    private fun markAsManuallyPaused(asin: String) {
        val manuallyPaused = getManuallyPausedAsins().toMutableSet()
        manuallyPaused.add(asin)
        prefs.edit().putStringSet(PREF_MANUALLY_PAUSED, manuallyPaused).apply()
        Log.d(TAG, "Marked $asin as manually paused")
    }

    /**
     * Remove manual pause marker (when user manually resumes or download completes)
     */
    private fun clearManuallyPaused(asin: String) {
        val manuallyPaused = getManuallyPausedAsins().toMutableSet()
        if (manuallyPaused.remove(asin)) {
            prefs.edit().putStringSet(PREF_MANUALLY_PAUSED, manuallyPaused).apply()
            Log.d(TAG, "Cleared manual pause marker for $asin")
        }
    }

    /**
     * Get set of manually paused ASINs
     */
    private fun getManuallyPausedAsins(): Set<String> {
        return prefs.getStringSet(PREF_MANUALLY_PAUSED, emptySet()) ?: emptySet()
    }

    /**
     * Resume pending tasks on app restart
     */
    private fun resumePendingTasks() {
        scope.launch {
            try {
                // List all incomplete downloads
                val listParams = JSONObject().apply {
                    put("db_path", dbPath)
                }

                val listResult = ExpoRustBridgeModule.nativeListDownloadTasks(listParams.toString())
                val parsed = parseJsonResponse(listResult)

                if (parsed["success"] == true) {
                    val data = parsed["data"] as? Map<*, *>
                    @Suppress("UNCHECKED_CAST")
                    val tasks = data?.get("tasks") as? List<Map<*, *>> ?: emptyList()

                    tasks.forEach { task ->
                        val status = task["status"] as? String
                        val asin = task["asin"] as? String ?: return@forEach
                        val taskId = task["task_id"] as? String ?: return@forEach

                        // Resume monitoring for incomplete downloads
                        if (status in listOf("queued", "downloading", "paused")) {
                            Log.d(TAG, "Resuming monitoring for $asin (status: $status)")
                            // Start monitoring (will need task details - simplified for now)
                            // TODO: Store task metadata in database or SharedPreferences
                        }
                    }
                }

                // Resume conversion manager tasks
                conversionManager.resumeAllPending()

            } catch (e: Exception) {
                Log.e(TAG, "Error resuming pending tasks", e)
            }
        }
    }

    /**
     * Set progress callback
     * Parameters: (asin, stage, percentage, bytesDownloaded, totalBytes)
     * Stage can be: "downloading", "decrypting", "copying"
     */
    fun setProgressCallback(callback: (String, String, Double, Long, Long) -> Unit) {
        this.progressCallback = callback
    }

    /**
     * Set completion callback
     */
    fun setCompletionCallback(callback: (String, String, String) -> Unit) {
        this.completionCallback = callback
    }

    /**
     * Set error callback
     */
    fun setErrorCallback(callback: (String, String, String) -> Unit) {
        this.errorCallback = callback
    }

    /**
     * Manually pause a download (will not auto-resume on WiFi)
     */
    suspend fun manuallyPauseDownload(asin: String, taskId: String): Boolean = withContext(Dispatchers.IO) {
        try {
            val pauseParams = JSONObject().apply {
                put("db_path", dbPath)
                put("task_id", taskId)
            }

            val result = ExpoRustBridgeModule.nativePauseDownload(pauseParams.toString())
            val parsed = parseJsonResponse(result)

            if (parsed["success"] == true) {
                markAsManuallyPaused(asin)
                Log.d(TAG, "Manually paused download: $asin")
                true
            } else {
                Log.e(TAG, "Failed to pause: ${parsed["error"]}")
                false
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error pausing download", e)
            false
        }
    }

    /**
     * Manually resume a download (clears manual pause marker)
     */
    suspend fun manuallyResumeDownload(asin: String, taskId: String): Boolean = withContext(Dispatchers.IO) {
        try {
            val resumeParams = JSONObject().apply {
                put("db_path", dbPath)
                put("task_id", taskId)
            }

            val result = ExpoRustBridgeModule.nativeResumeDownload(resumeParams.toString())
            val parsed = parseJsonResponse(result)

            if (parsed["success"] == true) {
                clearManuallyPaused(asin)
                Log.d(TAG, "Manually resumed download: $asin")
                true
            } else {
                Log.e(TAG, "Failed to resume: ${parsed["error"]}")
                false
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error resuming download", e)
            false
        }
    }

    /**
     * Stop all monitoring and conversion for an ASIN
     */
    fun stopMonitoring(asin: String) {
        monitoringJobs[asin]?.cancel()
        monitoringJobs.remove(asin)
        Log.d(TAG, "Stopped monitoring for $asin")
    }

    /**
     * Shutdown orchestrator
     */
    fun shutdown() {
        // Cancel network monitoring
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            networkCallback?.let {
                connectivityManager.unregisterNetworkCallback(it)
            }
        }

        // Cancel all monitoring jobs
        monitoringJobs.values.forEach { it.cancel() }
        monitoringJobs.clear()

        // Shutdown managers
        conversionManager.shutdown()
        scope.cancel()
    }

    // ========================================================================
    // Helper Methods
    // ========================================================================

    /**
     * Build file path for book using naming pattern from settings.
     * Defaults to author_series_book pattern.
     */
    private fun buildFilePathForBook(asin: String): String {
        return try {
            // Get naming pattern from SharedPreferences (default to author_series_book)
            val prefs = context.getSharedPreferences("app_settings", Context.MODE_PRIVATE)
            val namingPattern = prefs.getString("naming_pattern", "author_series_book") ?: "author_series_book"

            val params = JSONObject().apply {
                put("db_path", dbPath)
                put("asin", asin)
                put("naming_pattern", namingPattern)
            }

            val result = ExpoRustBridgeModule.nativeBuildFilePath(params.toString())
            val parsed = parseJsonResponse(result)

            if (parsed["success"] == true) {
                val data = parsed["data"] as? Map<*, *>
                data?.get("file_path") as? String ?: "$asin.m4b"
            } else {
                Log.w(TAG, "Failed to build file path for $asin: ${parsed["error"]}, using fallback")
                "$asin.m4b"
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error building file path for $asin", e)
            "$asin.m4b"  // Fallback to ASIN
        }
    }

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
     * Fetch book metadata from database by ASIN
     */
    private fun fetchBookMetadata(asin: String): Map<String, Any?>? {
        return try {
            val params = JSONObject().apply {
                put("db_path", dbPath)
                put("asin", asin)
            }

            val result = ExpoRustBridgeModule.nativeGetBookByAsin(params.toString())
            val parsed = parseJsonResponse(result)

            if (parsed["success"] == true) {
                val book = parsed["data"] as? Map<*, *>

                if (book != null) {
                    // Helper to convert JSONArray to comma-separated string
                    fun jsonArrayToString(value: Any?): String? {
                        return when (value) {
                            is org.json.JSONArray -> {
                                (0 until value.length())
                                    .mapNotNull { value.optString(it, null) }
                                    .filter { it.isNotEmpty() }
                                    .joinToString(", ")
                                    .takeIf { it.isNotEmpty() }
                            }
                            is List<*> -> value.mapNotNull { it?.toString() }.joinToString(", ").takeIf { it.isNotEmpty() }
                            is String -> value.takeIf { it.isNotEmpty() }
                            else -> null
                        }
                    }

                    // Return metadata map with proper field names
                    mapOf(
                        "title" to book["title"],
                        "subtitle" to book["subtitle"],
                        "description" to book["description"],
                        "authors" to jsonArrayToString(book["authors"]),
                        "narrators" to jsonArrayToString(book["narrators"]),
                        "publisher" to book["publisher"],
                        "series_name" to book["series_name"],
                        "series_sequence" to book["series_sequence"],
                        "date_published" to book["release_date"],
                        "language" to book["language"],
                        "picture_large" to book["cover_url"],
                        "audible_asin" to asin
                    )
                } else {
                    Log.w(TAG, "No book metadata found for ASIN: $asin")
                    null
                }
            } else {
                Log.w(TAG, "Book not found in database: $asin (${parsed["error"]})")
                null
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error fetching metadata for $asin", e)
            null
        }
    }

    private fun parseJsonResponse(jsonString: String): Map<String, Any?> {
        return try {
            val json = JSONObject(jsonString)
            val success = json.getBoolean("success")

            if (success) {
                val dataObj = json.getJSONObject("data")
                val dataMap = mutableMapOf<String, Any?>()

                dataObj.keys().forEach { key ->
                    val value = dataObj.get(key)
                    dataMap[key] = if (value == JSONObject.NULL) null else value
                }

                mapOf("success" to true, "data" to dataMap)
            } else {
                mapOf("success" to false, "error" to json.getString("error"))
            }
        } catch (e: Exception) {
            mapOf("success" to false, "error" to "Parse error: ${e.message}")
        }
    }
}
