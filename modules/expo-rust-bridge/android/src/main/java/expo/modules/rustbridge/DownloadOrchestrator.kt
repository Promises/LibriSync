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
 * - Handles FFmpeg-Kit decryption with metadata and cover art
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

    private val prefs: SharedPreferences = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())

    // Network monitoring
    private val connectivityManager = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    private var isWifiAvailable = false

    // Active download monitoring jobs
    private val monitoringJobs = mutableMapOf<String, Job>()

    // Callbacks
    private var progressCallback: ((String, String, Double, Long, Long, Long) -> Unit)? = null // (asin, stage, percentage, bytesDownloaded, totalBytes, etaSeconds)
    private var completionCallback: ((String, String, String) -> Unit)? = null // (asin, title, outputPath)
    private var errorCallback: ((String, String, String) -> Unit)? = null // (asin, title, error)

    init {
        setupNetworkMonitoring()
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
            val fileType = ((licenseData["file_type"] as? String) ?: "aaxc").lowercase()
            val fileExtension = ((licenseData["file_extension"] as? String)?.takeIf { it.isNotBlank() }
                ?: if (fileType == "mp3") "mp3" else "aax").lowercase()
            val isPlainAudio = fileType == "mp3"
            val aaxcKey = licenseData["aaxc_key"] as? String
            val aaxcIv = licenseData["aaxc_iv"] as? String
            if (!isPlainAudio && (aaxcKey.isNullOrEmpty() || aaxcIv.isNullOrEmpty())) {
                throw Exception("No AAXC key")
            }
            @Suppress("UNCHECKED_CAST")
            val requestHeaders = licenseData["request_headers"] as? Map<String, String>
                ?: mapOf("User-Agent" to "Audible/671 CFNetwork/1240.0.4 Darwin/20.6.0")

            Log.d(TAG, "License obtained. Type: $fileType, size: ${totalBytes / 1024 / 1024} MB")

            // Step 2: Prepare paths
            val cacheDir = context.cacheDir
            val audiobooksDir = File(cacheDir, "audiobooks")
            audiobooksDir.mkdirs()

            val encryptedPath = File(audiobooksDir, "$asin.$fileExtension").absolutePath
            val decryptedCachePath = if (isPlainAudio) {
                encryptedPath
            } else {
                File(audiobooksDir, "$asin.m4b").absolutePath
            }

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

            // Step 4: Store conversion keys in DB for retry capability
            storeConversionKeysInDb(taskId, aaxcKey.orEmpty(), aaxcIv.orEmpty(), outputDirectory)

            // Step 5: Start monitoring this download
            startMonitoringDownload(
                taskId = taskId,
                asin = asin,
                title = title,
                encryptedPath = encryptedPath,
                decryptedCachePath = decryptedCachePath,
                outputDirectory = outputDirectory,
                aaxcKey = aaxcKey.orEmpty(),
                aaxcIv = aaxcIv.orEmpty(),
                totalBytes = totalBytes,
                plainAudio = isPlainAudio
            )

            taskId
        } catch (e: Exception) {
            Log.e(TAG, "Failed to enqueue book", e)
            errorCallback?.invoke(asin, title, e.message ?: "Unknown error")
            throw e
        }
    }

    /**
     * Enqueue a LibriVox book for download (no license, no decryption)
     *
     * Uses the same Rust PersistentDownloadManager and monitoring pipeline as Audible,
     * but skips license fetching, decryption, and audio validation since LibriVox
     * files are plain MP3s.
     */
    suspend fun enqueueLibrivoxBook(
        librivoxId: String,
        title: String,
        author: String,
        downloadUrl: String,
        outputDirectory: String
    ): String = withContext(Dispatchers.IO) {
        val asin = "librivox_$librivoxId"
        Log.d(TAG, "Enqueueing LibriVox book: $asin - $title")

        try {
            // Prepare paths - preserve original file extension from URL
            val cacheDir = context.cacheDir
            val audiobooksDir = File(cacheDir, "audiobooks")
            audiobooksDir.mkdirs()

            // Detect extension from URL - check query params (archive.org uses &file=) and path
            val parsedUrl = Uri.parse(downloadUrl)
            val fileParam = parsedUrl.getQueryParameter("file")
            val urlForExt = fileParam ?: parsedUrl.lastPathSegment ?: downloadUrl
            val extension = urlForExt.substringAfterLast('.', "mp3").substringBefore('?').lowercase()
            val downloadPath = File(audiobooksDir, "$asin.$extension").absolutePath

            // Enqueue download in Rust manager (no license needed, no special headers)
            val enqueueParams = JSONObject().apply {
                put("db_path", dbPath)
                put("asin", asin)
                put("title", title)
                put("download_url", downloadUrl)
                put("total_bytes", 0) // Unknown until download starts
                put("download_path", downloadPath)
                put("output_path", downloadPath) // Same file, no conversion needed
                put("request_headers", JSONObject())
            }

            val enqueueResult = ExpoRustBridgeModule.nativeEnqueueDownload(enqueueParams.toString())
            val parsedEnqueue = parseJsonResponse(enqueueResult)

            if (parsedEnqueue["success"] != true) {
                throw Exception("Failed to enqueue: ${parsedEnqueue["error"]}")
            }

            val enqueueData = parsedEnqueue["data"] as? Map<*, *>
            val taskId = enqueueData?.get("task_id") as? String ?: throw Exception("No task ID")

            Log.d(TAG, "LibriVox download enqueued: $taskId")

            // Store output directory for the copy step (reuse conversion keys storage)
            storeConversionKeysInDb(taskId, "", "", outputDirectory)

            // Start monitoring (uses a simplified path that skips decryption)
            startMonitoringLibrivoxDownload(
                taskId = taskId,
                asin = asin,
                title = title,
                author = author,
                downloadPath = downloadPath,
                outputDirectory = outputDirectory
            )

            taskId
        } catch (e: Exception) {
            Log.e(TAG, "Failed to enqueue LibriVox book", e)
            errorCallback?.invoke(asin, title, e.message ?: "Unknown error")
            throw e
        }
    }

    /**
     * Monitor a LibriVox download and copy to final destination on completion.
     * Skips decryption and validation — MP3 files are ready to copy directly.
     */
    private fun startMonitoringLibrivoxDownload(
        taskId: String,
        asin: String,
        title: String,
        author: String,
        downloadPath: String,
        outputDirectory: String
    ) {
        monitoringJobs[asin]?.cancel()

        progressCallback?.invoke(asin, "downloading", 0.0, 0, 0, 0L)

        val job = scope.launch {
            try {
                val speedEta = SpeedEta()
                while (isActive) {
                    delay(2000)

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
                        val taskTotalBytes = (taskData?.get("total_bytes") as? Number)?.toLong() ?: 0L
                        val percentage = if (taskTotalBytes > 0) {
                            (bytesDownloaded.toDouble() / taskTotalBytes) * 100.0
                        } else 0.0

                        when (status) {
                            "downloading" -> {
                                speedEta.update(bytesDownloaded, taskTotalBytes)
                                progressCallback?.invoke(asin, "downloading", percentage, bytesDownloaded, taskTotalBytes, speedEta.etaSeconds)
                            }
                            "paused" -> {
                                // Continue monitoring, skip progress updates
                            }
                            "completed" -> {
                                Log.d(TAG, "LibriVox download completed for $asin, copying to destination")
                                try {
                                    triggerLibrivoxCopy(asin, title, downloadPath, outputDirectory, taskId)
                                } catch (e: CancellationException) {
                                    throw e
                                }
                                break
                            }
                            "failed" -> {
                                val error = taskData?.get("error") as? String ?: "Unknown error"
                                errorCallback?.invoke(asin, title, error)
                                break
                            }
                            "cancelled" -> break
                        }
                    } else {
                        Log.e(TAG, "Failed to check LibriVox status: ${parsedStatus["error"]}")
                        break
                    }
                }
            } catch (e: Exception) {
                if (e !is CancellationException) {
                    Log.e(TAG, "Error monitoring LibriVox download $asin", e)
                }
            } finally {
                monitoringJobs.remove(asin)
            }
        }

        monitoringJobs[asin] = job
    }

    /**
     * Copy a completed LibriVox download to the final SAF destination.
     * No decryption or validation needed.
     */
    private suspend fun triggerLibrivoxCopy(
        asin: String,
        title: String,
        downloadPath: String,
        outputDirectory: String,
        taskId: String
    ) = withContext(Dispatchers.IO) {
        try {
            updateTaskStatusInDb(taskId, "copying")
            progressCallback?.invoke(asin, "copying", 0.0, 0, 0, 0L)

            val finalPath = copyLibrivoxToFinalDestination(asin, title, downloadPath, outputDirectory)

            File(downloadPath).delete()

            updateTaskStatusInDb(taskId, "completed", finalPath)
            clearManuallyPaused(asin)
            completionCallback?.invoke(asin, title, finalPath)
        } catch (e: Exception) {
            Log.e(TAG, "LibriVox copy failed for $asin", e)
            updateTaskStatusWithError(taskId, "failed", e.message ?: "Copy failed")
            errorCallback?.invoke(asin, title, e.message ?: "Copy failed")
        }
    }

    /**
     * Copy or extract a LibriVox download to the user's SAF directory.
     * If the file is a zip, extracts audio files into Author/Title/.
     * Otherwise copies the single file directly.
     */
    private suspend fun copyLibrivoxToFinalDestination(
        asin: String,
        title: String,
        downloadPath: String,
        outputDirectory: String
    ): String = withContext(Dispatchers.IO) {
        val cachedFile = File(downloadPath)

        val treeUri = Uri.parse(outputDirectory)
        val docDir = if (outputDirectory.startsWith("content://")) {
            DocumentFile.fromTreeUri(context, treeUri)
                ?: throw Exception("Invalid SAF URI")
        } else null

        if (docDir != null && !docDir.canWrite()) {
            throw Exception("No write permission for SAF directory")
        }

        // Build directory path using naming pattern (Author/Title/)
        val filePath = buildFilePathForBook(asin)
        Log.d(TAG, "Using file path: $filePath")

        val pathParts = filePath.split('/')
        val directories = pathParts.dropLast(1)

        // Navigate/create subdirectories
        var safTargetDir: DocumentFile? = null
        var regularTargetPath: String? = null

        if (docDir != null) {
            var currentDir: DocumentFile = docDir
            for (dirName in directories) {
                val existing = currentDir.findFile(dirName)
                currentDir = if (existing != null && existing.isDirectory) {
                    existing
                } else {
                    currentDir.createDirectory(dirName)
                        ?: throw Exception("Failed to create directory: $dirName")
                }
            }
            safTargetDir = currentDir
        } else {
            val dir = File(outputDirectory, directories.joinToString("/"))
            dir.mkdirs()
            regularTargetPath = dir.absolutePath
        }

        val extension = cachedFile.extension.lowercase()

        val finalPath = if (extension == "zip") {
            extractZipToDirectory(cachedFile, safTargetDir, regularTargetPath, asin)
        } else {
            copySingleFileToDirectory(cachedFile, extension, pathParts.last(), safTargetDir, regularTargetPath, asin)
        }

        cachedFile.delete()
        Log.d(TAG, "LibriVox files saved to: $finalPath")
        finalPath
    }

    /**
     * Extract a zip file's audio contents into the target SAF directory.
     */
    private fun extractZipToDirectory(
        zipFile: File,
        safDir: DocumentFile?,
        regularDirPath: String?,
        asin: String
    ): String {
        val audioExts = listOf("mp3", "m4a", "m4b", "ogg", "flac", "opus", "wav")
        // Pre-count audio entries (reads the zip's central directory) so extraction
        // can report progress + ETA per file.
        val totalAudio = try {
            java.util.zip.ZipFile(zipFile).use { zf ->
                zf.entries().asSequence().count {
                    !it.isDirectory &&
                        File(it.name).name.substringAfterLast('.', "").lowercase() in audioExts
                }
            }
        } catch (e: Exception) {
            0
        }

        var extractedCount = 0
        var firstPath: String? = null
        val speedEta = SpeedEta()

        java.util.zip.ZipInputStream(zipFile.inputStream().buffered()).use { zis ->
            var entry = zis.nextEntry
            while (entry != null) {
                if (!entry.isDirectory) {
                    val entryName = File(entry.name).name // strip any directory prefix
                    val entryExt = entryName.substringAfterLast('.', "").lowercase()

                    // Only extract audio files
                    if (entryExt in listOf("mp3", "m4a", "m4b", "ogg", "flac", "opus", "wav")) {
                        if (safDir != null) {
                            // SAF path
                            safDir.findFile(entryName)?.delete()
                            val mimeType = when (entryExt) {
                                "mp3" -> "audio/mpeg"
                                "m4a", "m4b" -> "audio/mp4"
                                "ogg" -> "audio/ogg"
                                "flac" -> "audio/flac"
                                "opus" -> "audio/opus"
                                "wav" -> "audio/wav"
                                else -> "audio/*"
                            }
                            val outputFile = safDir.createFile(mimeType, entryName)
                                ?: throw Exception("Failed to create file: $entryName")
                            context.contentResolver.openOutputStream(outputFile.uri)?.use { out ->
                                zis.copyTo(out)
                            } ?: throw Exception("Failed to write: $entryName")
                            if (firstPath == null) firstPath = outputFile.uri.toString()
                        } else {
                            // Regular file path
                            val outputFile = File(regularDirPath!!, entryName)
                            outputFile.outputStream().use { out ->
                                zis.copyTo(out)
                            }
                            if (firstPath == null) firstPath = outputFile.absolutePath
                        }
                        extractedCount++
                        if (totalAudio > 0) {
                            speedEta.update(extractedCount.toLong(), totalAudio.toLong())
                            val pct = (extractedCount * 100.0 / totalAudio).coerceIn(0.0, 100.0)
                            progressCallback?.invoke(asin, "copying", pct, 0, 0, speedEta.etaSeconds)
                        }
                    }
                }
                zis.closeEntry()
                entry = zis.nextEntry
            }
        }

        Log.d(TAG, "Extracted $extractedCount audio files from zip")
        if (extractedCount == 0) throw Exception("No audio files found in zip")
        return firstPath!!
    }

    /**
     * Copy a single audio file into the target directory.
     */
    private fun copySingleFileToDirectory(
        sourceFile: File,
        extension: String,
        fileName: String,
        safDir: DocumentFile?,
        regularDirPath: String?,
        asin: String
    ): String {
        // Replace extension in filename
        val targetName = fileName.replaceAfterLast('.', extension)
        val mimeType = when (extension) {
            "mp3" -> "audio/mpeg"
            "m4a", "m4b" -> "audio/mp4"
            else -> "audio/*"
        }
        val totalBytes = sourceFile.length()
        val onCopyProgress: (Int, Int) -> Unit = { pct, eta ->
            progressCallback?.invoke(asin, "copying", pct.toDouble(), 0, 0, eta.toLong())
        }

        return if (safDir != null) {
            safDir.findFile(targetName)?.delete()
            val outputFile = safDir.createFile(mimeType, targetName)
                ?: throw Exception("Failed to create file: $targetName")
            context.contentResolver.openOutputStream(outputFile.uri)?.use { out ->
                sourceFile.inputStream().use { inp -> copyStreamWithProgress(inp, out, totalBytes, onCopyProgress) }
            } ?: throw Exception("Failed to write: $targetName")
            outputFile.uri.toString()
        } else {
            val outputFile = File(regularDirPath!!, targetName)
            outputFile.outputStream().use { out ->
                sourceFile.inputStream().use { inp -> copyStreamWithProgress(inp, out, totalBytes, onCopyProgress) }
            }
            outputFile.absolutePath
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
        totalBytes: Long,
        plainAudio: Boolean = false
    ) {
        // Cancel any existing monitoring for this ASIN
        monitoringJobs[asin]?.cancel()

        // Send initial progress notification (0%)
        progressCallback?.invoke(asin, "downloading", 0.0, 0, totalBytes, 0L)

        val job = scope.launch {
            try {
                val speedEta = SpeedEta()
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
                        val percentage = if (taskTotalBytes > 0) {
                            (bytesDownloaded.toDouble() / taskTotalBytes) * 100.0
                        } else {
                            0.0
                        }

                        Log.d(TAG, "Download $asin: $status ($percentage%)")

                        when (status) {
                            "downloading" -> {
                                // Send progress notification only while downloading
                                speedEta.update(bytesDownloaded, taskTotalBytes)
                                progressCallback?.invoke(asin, "downloading", percentage, bytesDownloaded, taskTotalBytes, speedEta.etaSeconds)
                            }
                            "paused" -> {
                                Log.d(TAG, "Download paused for $asin - will resume monitoring when unpaused")
                                // Continue monitoring but don't send progress notifications
                                // This allows detection of resume events
                            }
                            "completed" -> {
                                Log.d(TAG, "Download completed! Finalizing $asin")

                                // The Rust download slot is now free; start the next
                                // queued download (workers can't advance the queue).
                                kickDownloadQueue()

                                // Trigger conversion or plain MP3 copy (cancellable via coroutine scope)
                                try {
                                    if (plainAudio) {
                                        triggerPlainAudioCopy(asin, title, encryptedPath, outputDirectory, taskId)
                                    } else {
                                        triggerConversion(
                                            asin, title, encryptedPath, decryptedCachePath,
                                            outputDirectory, aaxcKey, aaxcIv, taskId
                                        )
                                    }
                                } catch (e: CancellationException) {
                                    Log.d(TAG, "Finalization cancelled for $asin")
                                    throw e // Re-throw to exit the monitoring loop
                                }

                                // Stop monitoring
                                break
                            }
                            "failed" -> {
                                val error = taskData?.get("error") as? String ?: "Unknown error"
                                Log.e(TAG, "Download failed for $asin: $error")
                                errorCallback?.invoke(asin, title, error)
                                kickDownloadQueue()
                                break
                            }
                            "cancelled" -> {
                                Log.d(TAG, "Download cancelled for $asin")
                                kickDownloadQueue()
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
     * Ask the Rust manager to start any queued downloads that now fit within the
     * concurrency limit. Download workers can't advance the queue themselves, so
     * we kick it whenever a download reaches a terminal state.
     */
    fun kickDownloadQueue() {
        try {
            val params = JSONObject().apply { put("db_path", dbPath) }
            ExpoRustBridgeModule.nativeStartPendingDownloads(params.toString())
        } catch (e: Exception) {
            Log.e(TAG, "Failed to start pending downloads", e)
        }
    }

    /**
     * Copy a completed plain Audible MP3 download to the final destination.
     */
    private suspend fun triggerPlainAudioCopy(
        asin: String,
        title: String,
        downloadPath: String,
        outputDirectory: String,
        taskId: String
    ) = withContext(Dispatchers.IO) {
        try {
            updateTaskStatusInDb(taskId, "copying")
            progressCallback?.invoke(asin, "copying", 0.0, 0, 0, 0L)

            val finalPath = copyLibrivoxToFinalDestination(asin, title, downloadPath, outputDirectory)

            updateTaskStatusInDb(taskId, "completed", finalPath)
            clearManuallyPaused(asin)
            completionCallback?.invoke(asin, title, finalPath)
        } catch (e: Exception) {
            Log.e(TAG, "Plain audio copy failed for $asin", e)
            updateTaskStatusWithError(taskId, "failed", e.message ?: "Copy failed")
            errorCallback?.invoke(asin, title, e.message ?: "Copy failed")
        }
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
        aaxcIv: String,
        taskId: String? = null
    ) = withContext(Dispatchers.IO) {
        // Resolve task ID outside try so it's available in catch
        val resolvedTaskId = taskId ?: findTaskIdForAsin(asin)

        try {
            Log.d(TAG, "Starting conversion for $asin...")

            // Persist decrypting stage to DB
            resolvedTaskId?.let { updateTaskStatusInDb(it, "decrypting") }

            // Notify decrypting stage
            progressCallback?.invoke(asin, "decrypting", 0.0, 0, 0, 0L)

            // Fetch metadata from database
            val metadata = fetchBookMetadata(asin)

            // Download cover art if available
            var coverArtPath: String? = null
            if (metadata != null) {
                val coverUrl = metadata["picture_large"] as? String
                if (coverUrl != null && coverUrl.isNotEmpty()) {
                    try {
                        val coverFile = File.createTempFile("cover_", ".jpg")
                        // Timeouts: a hung cover fetch must not stall the whole pipeline.
                        val conn = (java.net.URL(coverUrl).openConnection() as java.net.HttpURLConnection).apply {
                            connectTimeout = 10_000
                            readTimeout = 15_000
                        }
                        conn.inputStream.use { input ->
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

            // Probe total duration from the encrypted input's (unencrypted) container
            // metadata so decrypt can report real progress + ETA. 0 = unknown -> indeterminate.
            val totalDurationSec = com.arthenica.ffmpegkit.FFprobeKit
                .getMediaInformation(encryptedPath)
                .mediaInformation?.duration?.toDoubleOrNull() ?: 0.0

            // FFmpeg statistics stream: time = ms of media processed, speed = realtime multiplier.
            // Throttle to whole-percent steps to avoid hammering the notification.
            var lastReportedPct = -1
            com.arthenica.ffmpegkit.FFmpegKitConfig.enableStatisticsCallback { stat ->
                if (totalDurationSec > 0.0) {
                    val processedSec = stat.time.toDouble() / 1000.0
                    val pct = ((processedSec / totalDurationSec).coerceIn(0.0, 1.0) * 100.0).toInt()
                    if (pct > lastReportedPct) {
                        lastReportedPct = pct
                        val speed = stat.speed
                        val etaSec = if (speed > 0.0)
                            ((totalDurationSec - processedSec) / speed).toLong().coerceAtLeast(0L)
                        else 0L
                        progressCallback?.invoke(asin, "decrypting", pct.toDouble(), 0, 0, etaSec)
                    }
                }
            }

            val session = try {
                com.arthenica.ffmpegkit.FFmpegKit.execute(command)
            } finally {
                // Global callback: clear so later validation/probe ffmpeg calls don't feed it.
                com.arthenica.ffmpegkit.FFmpegKitConfig.enableStatisticsCallback(null)
            }

            if (!com.arthenica.ffmpegkit.ReturnCode.isSuccess(session.returnCode)) {
                val ffmpegOutput = session.allLogsAsString
                Log.e(TAG, "FFmpeg failed with return code: ${session.returnCode}")
                Log.e(TAG, "FFmpeg output: $ffmpegOutput")
                throw Exception(ffmpegFailureMessage(ffmpegOutput))
            }

            Log.d(TAG, "Conversion complete for $asin (with metadata + cover art)")

            // CRITICAL: Validate audio file for corruption
            Log.d(TAG, "Validating audio file integrity for $asin...")
            resolvedTaskId?.let { updateTaskStatusInDb(it, "validating") }
            progressCallback?.invoke(asin, "validating", 0.0, 0, 0, 0L)

            val validationResult = validateAudioFile(decryptedCachePath, asin)

            if (!validationResult.isValid) {
                Log.e(TAG, "Audio validation FAILED for $asin:")
                Log.e(TAG, "  Error count: ${validationResult.errorCount}")
                Log.e(TAG, "  Duration: ${validationResult.duration}s")
                Log.e(TAG, "  Message: ${validationResult.errorMessage}")

                // Delete corrupt files
                File(decryptedCachePath).delete()
                File(encryptedPath).delete()

                throw Exception("Audio file validation failed: Corruption detected. ${validationResult.errorMessage}")
            }

            Log.d(TAG, "✓ Audio validation PASSED for $asin (${validationResult.duration}s, 0 errors)")

            // Notify copying stage
            resolvedTaskId?.let { updateTaskStatusInDb(it, "copying") }
            progressCallback?.invoke(asin, "copying", 0.0, 0, 0, 0L)

            // Copy to final destination
            val finalPath = copyToFinalDestination(asin, title, decryptedCachePath, outputDirectory, coverArtPath)

            // Cleanup encrypted file
            File(encryptedPath).delete()

            // Cleanup cover art temp file
            coverArtPath?.let { File(it).delete() }

            // Mark as completed in DB with the final SAF/file path
            resolvedTaskId?.let { updateTaskStatusInDb(it, "completed", finalPath) }

        } catch (e: Exception) {
            Log.e(TAG, "Conversion failed for $asin", e)
            // Mark as failed in DB with error
            resolvedTaskId?.let { updateTaskStatusWithError(it, "failed", e.message ?: "Conversion failed") }
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
        outputDirectory: String,
        coverArtPath: String?
    ): String = withContext(Dispatchers.IO) {
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
            val sourceExtension = cachedFile.extension.lowercase().ifBlank { "m4b" }
            val targetFileName = if (fileName.contains('.')) {
                fileName.replaceAfterLast('.', sourceExtension)
            } else {
                "$fileName.$sourceExtension"
            }
            val mimeType = when (sourceExtension) {
                "mp3" -> "audio/mpeg"
                "m4a", "m4b", "mp4" -> "audio/mp4"
                else -> "audio/*"
            }

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
            currentDir.findFile(targetFileName)?.delete()

            // Create new file
            val outputFile = currentDir.createFile(mimeType, targetFileName)
                ?: currentDir.createFile("audio/*", targetFileName)
                ?: throw Exception("Failed to create file in SAF directory")

            Log.d(TAG, "Copying to SAF: ${outputFile.uri}")

            // Copy (with progress + ETA — large M4B over SAF is slow)
            val totalBytes = cachedFile.length()
            context.contentResolver.openOutputStream(outputFile.uri)?.use { outputStream ->
                cachedFile.inputStream().use { inputStream ->
                    copyStreamWithProgress(inputStream, outputStream, totalBytes) { pct, eta ->
                        progressCallback?.invoke(asin, "copying", pct.toDouble(), 0, 0, eta.toLong())
                    }
                }
            } ?: throw Exception("Failed to open output stream")

            finalPath = outputFile.uri.toString()

            // Delete cache file
            cachedFile.delete()

            // Save Smart Audiobook Player cover if enabled
            if (coverArtPath != null) {
                try {
                    val prefs = context.getSharedPreferences("app_settings", Context.MODE_PRIVATE)
                    val smartPlayerCoverEnabled = prefs.getString("smart_player_cover_enabled", "false") == "true"

                    if (smartPlayerCoverEnabled) {
                        Log.d(TAG, "Creating Smart Audiobook Player cover (EmbeddedCover.jpg)")
                        saveSmartPlayerCover(coverArtPath, currentDir)
                    }
                } catch (e: Exception) {
                    Log.w(TAG, "Failed to save Smart Audiobook Player cover: ${e.message}")
                }
            }
        }

        Log.d(TAG, "Complete! Final path: $finalPath")

        // Clear manual pause marker on completion
        clearManuallyPaused(asin)

        completionCallback?.invoke(asin, title, finalPath)

        finalPath
    }

    /**
     * Save cover art as EmbeddedCover.jpg (500x500) for Smart Audiobook Player
     */
    private fun saveSmartPlayerCover(coverArtPath: String, targetDir: DocumentFile) {
        try {
            // Load cover image
            val coverFile = File(coverArtPath)
            val originalBitmap = android.graphics.BitmapFactory.decodeFile(coverArtPath)
                ?: throw Exception("Failed to decode cover image")

            // Resize to 500x500
            val resizedBitmap = android.graphics.Bitmap.createScaledBitmap(
                originalBitmap,
                500,
                500,
                true
            )

            // Delete existing EmbeddedCover.jpg if present
            targetDir.findFile("EmbeddedCover.jpg")?.delete()

            // Create new file
            val embeddedCover = targetDir.createFile("image/jpeg", "EmbeddedCover.jpg")
                ?: throw Exception("Failed to create EmbeddedCover.jpg")

            // Write JPEG
            context.contentResolver.openOutputStream(embeddedCover.uri)?.use { outputStream ->
                resizedBitmap.compress(android.graphics.Bitmap.CompressFormat.JPEG, 90, outputStream)
            } ?: throw Exception("Failed to open output stream for EmbeddedCover.jpg")

            // Cleanup
            originalBitmap.recycle()
            resizedBitmap.recycle()

            Log.d(TAG, "Saved EmbeddedCover.jpg (500x500) to ${embeddedCover.uri}")
        } catch (e: Exception) {
            Log.w(TAG, "Error saving Smart Player cover: ${e.message}")
        }
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
     * Pause downloads before the OS removes foreground execution time.
     */
    suspend fun pauseActiveDownloadsForServiceTimeout() {
        pauseAllActiveDownloads()
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

            } catch (e: Exception) {
                Log.e(TAG, "Error resuming pending tasks", e)
            }
        }
    }

    /**
     * Set progress callback
     * Parameters: (asin, stage, percentage, bytesDownloaded, totalBytes, etaSeconds)
     * Stage can be: "downloading", "decrypting", "copying"
     * etaSeconds is 0 when unknown (only "decrypting" currently reports it)
     */
    fun setProgressCallback(callback: (String, String, Double, Long, Long, Long) -> Unit) {
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

        // Cleanup
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
            val podcastNamingPattern = prefs.getString("podcast_naming_pattern", "podcast_episode_folder")
                ?: "podcast_episode_folder"

            val params = JSONObject().apply {
                put("db_path", dbPath)
                put("asin", asin)
                put("naming_pattern", namingPattern)
                put("podcast_naming_pattern", podcastNamingPattern)
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

    /**
     * Validate audio file for corruption
     *
     * Checks multiple sample points throughout the file for AAC decode errors.
     * Returns validation result with error count and details.
     */
    private suspend fun validateAudioFile(filePath: String, asin: String): AudioValidationResult = withContext(Dispatchers.IO) {
        try {
            Log.d(TAG, "Validating audio file: $filePath")

            // Step 1: Get file duration using FFprobe
            val probeSession = com.arthenica.ffmpegkit.FFprobeKit.getMediaInformation(filePath)
            val duration = probeSession.mediaInformation?.duration?.toDoubleOrNull() ?: 0.0

            if (duration <= 0) {
                Log.e(TAG, "Invalid duration: $duration")
                return@withContext AudioValidationResult(
                    isValid = false,
                    errorCount = -1,
                    errorMessage = "Could not determine file duration",
                    duration = 0.0
                )
            }

            Log.d(TAG, "File duration: ${duration}s (${duration / 3600}h)")

            // Step 2: Sample multiple points in the file
            // Check: 30s, 25%, 50%, 75%, end-30s
            val samplePoints = listOf(
                30.0,                    // Start (30 seconds in)
                duration * 0.25,         // 25%
                duration * 0.50,         // 50%
                duration * 0.75,         // 75%
                maxOf(duration - 30, 60.0) // Near end (or 60s if file is short)
            ).distinct().sorted()

            // Validation depth is user-configurable: "full" (all points), "quick" (ends
            // only) or "off" (skip). Trades corruption-detection thoroughness for speed.
            val validationLevel = context.getSharedPreferences("app_settings", Context.MODE_PRIVATE)
                .getString("validation_level", "full") ?: "full"
            if (validationLevel == "off") {
                Log.d(TAG, "Validation skipped (setting=off) for $asin")
                progressCallback?.invoke(asin, "validating", 100.0, 0, 0, 0L)
                return@withContext AudioValidationResult(
                    isValid = true, errorCount = 0,
                    errorMessage = "Validation skipped by setting", duration = duration
                )
            }
            val effectiveSamplePoints = if (validationLevel == "quick")
                listOf(samplePoints.first(), samplePoints.last()).distinct()
            else samplePoints

            Log.d(TAG, "Sampling ${effectiveSamplePoints.size} points ($validationLevel): ${effectiveSamplePoints.map { "%.1fmin".format(it / 60) }}")

            var totalErrors = 0
            val sampleResults = mutableListOf<String>()
            val totalSamples = effectiveSamplePoints.size
            val testDuration = 10 // seconds decoded per sample

            // Validation cost is dominated by seeking into a huge file, which emits no
            // FFmpeg statistics — so drive smooth progress + ETA from a timer, seeded
            // BEFORE the first sample finishes (a novice must see it isn't hung), then
            // refined by each real sample's measured duration.
            val completedSamples = java.util.concurrent.atomic.AtomicInteger(0)
            val sampleStartMs = java.util.concurrent.atomic.AtomicLong(System.currentTimeMillis())
            val avgSampleMs = java.util.concurrent.atomic.AtomicLong(4000L) // seed ~4s/sample

            val progressTicker = launch {
                var lastPct = -1
                while (isActive) {
                    val done = completedSamples.get()
                    val avg = avgSampleMs.get().toDouble()
                    val sampleElapsed = (System.currentTimeMillis() - sampleStartMs.get()).toDouble()
                    val subFrac = (sampleElapsed / avg).coerceIn(0.0, 0.99)
                    val overall = ((done + subFrac) / totalSamples).coerceIn(0.0, 0.999)
                    val pct = (overall * 100.0).toInt()
                    if (pct != lastPct) {
                        lastPct = pct
                        val remaining = (totalSamples - (done + subFrac)).coerceAtLeast(0.0)
                        val etaSec = (remaining * avg / 1000.0).toLong().coerceAtLeast(0L)
                        progressCallback?.invoke(asin, "validating", pct.toDouble(), 0, 0, etaSec)
                    }
                    delay(400)
                }
            }

            try {
                // Step 3: Check each sample point for errors
                for ((index, timestamp) in effectiveSamplePoints.withIndex()) {
                    sampleStartMs.set(System.currentTimeMillis())
                    val command = "-v error -ss $timestamp -i \"$filePath\" -t $testDuration -f null -"

                    val session = com.arthenica.ffmpegkit.FFmpegKit.execute(command)
                    val output = session.allLogsAsString

                    // Count error lines
                    val errors = output.lines().count {
                        it.contains("Error", ignoreCase = true) ||
                        it.contains("Invalid data", ignoreCase = true)
                    }

                    totalErrors += errors
                    val statusMark = if (errors == 0) "✓" else "✗ $errors errors"
                    val timestampStr = formatTimestamp(timestamp.toLong())
                    sampleResults.add("  [$timestampStr] $statusMark")

                    // Refine the per-sample estimate from the measured time.
                    val took = System.currentTimeMillis() - sampleStartMs.get()
                    avgSampleMs.set(
                        if (index == 0) took.coerceAtLeast(250L)
                        else (0.6 * avgSampleMs.get() + 0.4 * took).toLong().coerceAtLeast(250L)
                    )
                    completedSamples.set(index + 1)

                    Log.d(TAG, "Sample ${index + 1}/$totalSamples at $timestampStr: $errors errors (${took}ms)")

                    // Early exit if we find significant corruption
                    if (errors > 50) {
                        Log.w(TAG, "High error count detected at $timestampStr, stopping validation")
                        break
                    }
                }
            } finally {
                progressTicker.cancel()
                // Ensure the bar reaches 100% even if the loop exited early.
                progressCallback?.invoke(asin, "validating", 100.0, 0, 0, 0L)
            }

            // Step 4: Determine if file is valid
            val isValid = totalErrors == 0
            val errorMessage = if (isValid) {
                "Audio file validated successfully"
            } else {
                "Audio corruption detected: $totalErrors total errors\n${sampleResults.joinToString("\n")}"
            }

            Log.d(TAG, "Validation result for $asin: ${if (isValid) "VALID" else "CORRUPT"} ($totalErrors errors)")

            AudioValidationResult(
                isValid = isValid,
                errorCount = totalErrors,
                errorMessage = errorMessage,
                duration = duration,
                samplePoints = sampleResults
            )

        } catch (e: Exception) {
            Log.e(TAG, "Error validating audio file", e)
            AudioValidationResult(
                isValid = false,
                errorCount = -1,
                errorMessage = "Validation failed: ${e.message}",
                duration = 0.0
            )
        }
    }

    /**
     * Format seconds to HH:MM:SS timestamp
     */
    private fun formatTimestamp(seconds: Long): String {
        val hours = seconds / 3600
        val minutes = (seconds % 3600) / 60
        val secs = seconds % 60
        return "%02d:%02d:%02d".format(hours, minutes, secs)
    }

    /**
     * Audio validation result
     */
    data class AudioValidationResult(
        val isValid: Boolean,
        val errorCount: Int,
        val errorMessage: String,
        val duration: Double,
        val samplePoints: List<String> = emptyList()
    )

    /**
     * Retry conversion for a failed download that has cached .aax file and stored keys
     */
    suspend fun retryConversion(asin: String): Boolean = withContext(Dispatchers.IO) {
        try {
            // Find the task for this ASIN by parsing the raw JSON response
            val listParams = JSONObject().apply {
                put("db_path", dbPath)
            }
            val listResult = ExpoRustBridgeModule.nativeListDownloadTasks(listParams.toString())
            val json = JSONObject(listResult)

            if (!json.getBoolean("success")) {
                Log.e(TAG, "Failed to list tasks for retry: ${json.optString("error")}")
                return@withContext false
            }

            val dataObj = json.getJSONObject("data")
            val tasksArray = dataObj.getJSONArray("tasks")

            // Find the failed task for this ASIN
            var taskObj: JSONObject? = null
            for (i in 0 until tasksArray.length()) {
                val t = tasksArray.getJSONObject(i)
                if (t.getString("asin") == asin && t.getString("status") == "failed") {
                    taskObj = t
                    break
                }
            }

            if (taskObj == null) {
                Log.e(TAG, "No failed task found for ASIN: $asin")
                return@withContext false
            }

            val taskId = taskObj.getString("task_id")
            val title = taskObj.optString("title", asin)
            val aaxcKey = taskObj.optString("aaxc_key", null)
            val aaxcIv = taskObj.optString("aaxc_iv", null)
            val outputDirectory = taskObj.optString("output_directory", null)

            if (aaxcKey == null || aaxcIv == null || outputDirectory == null) {
                Log.e(TAG, "Missing conversion keys for retry: key=$aaxcKey, iv=$aaxcIv, dir=$outputDirectory")
                return@withContext false
            }

            // Check if encrypted file still exists
            val cacheDir = context.cacheDir
            val audiobooksDir = File(cacheDir, "audiobooks")
            val encryptedPath = File(audiobooksDir, "$asin.aax").absolutePath
            val decryptedCachePath = File(audiobooksDir, "$asin.m4b").absolutePath

            if (!File(encryptedPath).exists()) {
                Log.e(TAG, "Encrypted file not found for retry: $encryptedPath")
                updateTaskStatusWithError(taskId, "failed", "Cached file not found - re-download required")
                return@withContext false
            }

            // Delete any corrupt decrypted file from previous attempt
            File(decryptedCachePath).delete()

            Log.d(TAG, "Retrying conversion for $asin (taskId=$taskId)")

            // Trigger conversion
            triggerConversion(
                asin, title, encryptedPath, decryptedCachePath,
                outputDirectory, aaxcKey, aaxcIv, taskId
            )

            true
        } catch (e: Exception) {
            Log.e(TAG, "Error retrying conversion for $asin", e)
            false
        }
    }

    /**
     * Update task status in the database via JNI
     */
    private fun updateTaskStatusInDb(taskId: String, status: String, outputPath: String? = null) {
        try {
            val params = JSONObject().apply {
                put("db_path", dbPath)
                put("task_id", taskId)
                put("status", status)
                outputPath?.let { put("output_path", it) }
            }
            ExpoRustBridgeModule.nativeUpdateDownloadTaskStatus(params.toString())
            Log.d(TAG, "Updated task $taskId status to $status in DB")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to update task status in DB: ${e.message}")
        }
    }

    /**
     * Update task status with error message in the database via JNI
     */
    private fun updateTaskStatusWithError(taskId: String, status: String, error: String) {
        try {
            val params = JSONObject().apply {
                put("db_path", dbPath)
                put("task_id", taskId)
                put("status", status)
                put("error", error)
            }
            ExpoRustBridgeModule.nativeUpdateDownloadTaskStatus(params.toString())
            Log.d(TAG, "Updated task $taskId status to $status with error in DB")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to update task status with error in DB: ${e.message}")
        }
    }

    /**
     * Store conversion keys in the database for retry capability
     */
    private fun storeConversionKeysInDb(taskId: String, aaxcKey: String, aaxcIv: String, outputDirectory: String) {
        try {
            val params = JSONObject().apply {
                put("db_path", dbPath)
                put("task_id", taskId)
                put("aaxc_key", aaxcKey)
                put("aaxc_iv", aaxcIv)
                put("output_directory", outputDirectory)
            }
            ExpoRustBridgeModule.nativeStoreConversionKeys(params.toString())
            Log.d(TAG, "Stored conversion keys for task $taskId")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to store conversion keys: ${e.message}")
        }
    }

    /**
     * Find the task ID for an ASIN from the database
     */
    private fun findTaskIdForAsin(asin: String): String? {
        return try {
            val listParams = JSONObject().apply {
                put("db_path", dbPath)
            }
            val listResult = ExpoRustBridgeModule.nativeListDownloadTasks(listParams.toString())
            val parsed = parseJsonResponse(listResult)

            if (parsed["success"] == true) {
                val data = parsed["data"] as? Map<*, *>
                @Suppress("UNCHECKED_CAST")
                val tasks = data?.get("tasks") as? List<Map<*, *>> ?: emptyList()
                tasks.find { it["asin"] == asin }?.get("task_id") as? String
            } else {
                null
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error finding task ID for $asin", e)
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
