package expo.modules.rustbridge.tasks

import android.content.Context
import android.content.SharedPreferences
import android.net.Uri
import android.util.Log
import androidx.documentfile.provider.DocumentFile
import expo.modules.rustbridge.ExpoRustBridgeModule
import expo.modules.rustbridge.copyStreamWithProgress
import expo.modules.rustbridge.SpeedEta
import expo.modules.rustbridge.ffmpegFailureMessage
import expo.modules.rustbridge.validateAudioFile
import kotlinx.coroutines.*
import org.json.JSONObject
import java.io.File

/**
 * Worker for handling download tasks
 *
 * Migrated from DownloadOrchestrator - handles:
 * - Download lifecycle management
 * - Progress monitoring
 * - FFmpeg-Kit decryption
 * - File copying to SAF directory
 * - Manual pause tracking
 */
class DownloadWorker(
    private val context: Context,
    private val manager: BackgroundTaskManager
) {
    companion object {
        private const val TAG = "DownloadWorker"
        private const val PREFS_NAME = "download_worker_prefs"
        private const val PREF_MANUALLY_PAUSED = "manually_paused_asins"
    }

    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
    private val prefs: SharedPreferences = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    // Active monitoring jobs
    private val monitoringJobs = mutableMapOf<String, Job>()

    // Per-ASIN cancel flag + the active FFmpeg session id, so a specific in-flight
    // decrypt/validate/copy can be aborted on its own — FFmpegKit.cancel(sessionId) instead of
    // FFmpegKit.cancel() (which would kill every parallel conversion). Mirrors DownloadOrchestrator.
    private val cancelledConversions = java.util.concurrent.ConcurrentHashMap.newKeySet<String>()
    private val activeFfmpegSessions = java.util.concurrent.ConcurrentHashMap<String, Long>()

    /**
     * Execute a download task
     */
    suspend fun execute(task: Task) = withContext(Dispatchers.IO) {
        try {
            val asin = task.getMetadataString(DownloadTaskMetadata.ASIN) ?: throw Exception("No ASIN")
            val title = task.getMetadataString(DownloadTaskMetadata.TITLE) ?: throw Exception("No title")
            val accountJson = task.getMetadataString("account_json") ?: throw Exception("No account")
            val outputDir = task.getMetadataString(DownloadTaskMetadata.OUTPUT_DIR) ?: throw Exception("No output directory")
            val quality = task.getMetadataString("quality") ?: "High"

            Log.d(TAG, "Executing download task: $asin - $title")

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

            // Update task metadata
            manager.updateTaskMetadata(task.id, mapOf(
                DownloadTaskMetadata.TOTAL_BYTES to totalBytes,
                DownloadTaskMetadata.ENCRYPTED_PATH to encryptedPath,
                DownloadTaskMetadata.DECRYPTED_PATH to decryptedCachePath,
                DownloadTaskMetadata.FILE_TYPE to fileType,
                DownloadTaskMetadata.FILE_EXTENSION to fileExtension,
                DownloadTaskMetadata.AAXC_KEY to aaxcKey.orEmpty(),
                DownloadTaskMetadata.AAXC_IV to aaxcIv.orEmpty()
            ))

            // Step 3: Enqueue download in Rust manager
            val enqueueParams = JSONObject().apply {
                put("db_path", manager.getDbPath())
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
            val rustTaskId = enqueueData?.get("task_id") as? String ?: throw Exception("No task ID")

            // Update task metadata with Rust task ID
            manager.updateTaskMetadata(task.id, mapOf(
                DownloadTaskMetadata.RUST_TASK_ID to rustTaskId
            ))

            Log.d(TAG, "Download enqueued in Rust: $rustTaskId")

            // Step 4: Start monitoring
            startMonitoring(
                task,
                rustTaskId,
                encryptedPath,
                decryptedCachePath,
                outputDir,
                aaxcKey.orEmpty(),
                aaxcIv.orEmpty(),
                totalBytes,
                isPlainAudio
            )

        } catch (e: Exception) {
            Log.e(TAG, "Failed to execute download task", e)
            task.status = TaskStatus.FAILED
            task.error = e.message
            task.completedAt = java.util.Date()
            manager.emitEvent(TaskEvent.TaskFailed(task, e.message ?: "Unknown error"))
            manager.unregisterActiveTask(task.id)
        }
    }

    /**
     * Start monitoring a download
     */
    private fun startMonitoring(
        task: Task,
        rustTaskId: String,
        encryptedPath: String,
        decryptedCachePath: String,
        outputDirectory: String,
        aaxcKey: String,
        aaxcIv: String,
        totalBytes: Long,
        plainAudio: Boolean = false
    ) {
        val asin = task.getMetadataString(DownloadTaskMetadata.ASIN) ?: return
        val title = task.getMetadataString(DownloadTaskMetadata.TITLE) ?: return

        // Cancel any existing monitoring
        monitoringJobs[task.id]?.cancel()

        // Send initial progress (0%)
        scope.launch {
            manager.emitEvent(TaskEvent.DownloadProgress(
                taskId = task.id,
                asin = asin,
                title = title,
                stage = "downloading",
                percentage = 0,
                bytesDownloaded = 0,
                totalBytes = totalBytes
            ))
        }

        val job = scope.launch {
            try {
                val speedEta = SpeedEta()
                while (isActive) {
                    delay(2000) // Poll every 2 seconds

                    // Check download status
                    val statusParams = JSONObject().apply {
                        put("db_path", manager.getDbPath())
                        put("task_id", rustTaskId)
                    }

                    val statusResult = ExpoRustBridgeModule.nativeGetDownloadTask(statusParams.toString())
                    val parsedStatus = parseJsonResponse(statusResult)

                    if (parsedStatus["success"] == true) {
                        val taskData = parsedStatus["data"] as? Map<*, *>
                        val status = taskData?.get("status") as? String
                        val bytesDownloaded = (taskData?.get("bytes_downloaded") as? Number)?.toLong() ?: 0L
                        val taskTotalBytes = (taskData?.get("total_bytes") as? Number)?.toLong() ?: totalBytes
                        val percentage = if (taskTotalBytes > 0) {
                            ((bytesDownloaded.toDouble() / taskTotalBytes) * 100.0).toInt()
                        } else 0

                        Log.d(TAG, "Download $asin: $status ($percentage%)")

                        when (status) {
                            "downloading" -> {
                                speedEta.update(bytesDownloaded, taskTotalBytes)
                                // Update task metadata
                                manager.updateTaskMetadata(task.id, mapOf(
                                    DownloadTaskMetadata.BYTES_DOWNLOADED to bytesDownloaded,
                                    DownloadTaskMetadata.PERCENTAGE to percentage,
                                    DownloadTaskMetadata.STAGE to "downloading",
                                    DownloadTaskMetadata.ETA_SECONDS to speedEta.etaSeconds.toInt()
                                ))

                                // Emit progress event
                                manager.emitEvent(TaskEvent.DownloadProgress(
                                    taskId = task.id,
                                    asin = asin,
                                    title = title,
                                    stage = "downloading",
                                    percentage = percentage,
                                    bytesDownloaded = bytesDownloaded,
                                    totalBytes = taskTotalBytes
                                ))
                            }
                            "paused" -> {
                                Log.d(TAG, "Download paused for $asin")
                                task.status = TaskStatus.PAUSED
                                manager.emitEvent(TaskEvent.TaskPaused(task))
                                // Continue monitoring to detect resume
                            }
                            "completed" -> {
                                Log.d(TAG, "Download completed! Finalizing $asin")

                                // Trigger conversion or plain MP3 copy
                                if (plainAudio) {
                                    completePlainAudioDownload(task, encryptedPath, outputDirectory)
                                } else {
                                    triggerConversion(task, encryptedPath, decryptedCachePath, outputDirectory, aaxcKey, aaxcIv)
                                }

                                // Stop monitoring
                                break
                            }
                            "failed" -> {
                                val error = taskData?.get("error") as? String ?: "Unknown error"
                                Log.e(TAG, "Download failed for $asin: $error")
                                task.status = TaskStatus.FAILED
                                task.error = error
                                task.completedAt = java.util.Date()
                                manager.emitEvent(TaskEvent.TaskFailed(task, error))
                                manager.unregisterActiveTask(task.id)
                                break
                            }
                            "cancelled" -> {
                                Log.d(TAG, "Download cancelled for $asin")
                                task.status = TaskStatus.CANCELLED
                                task.completedAt = java.util.Date()
                                manager.emitEvent(TaskEvent.TaskCancelled(task))
                                manager.unregisterActiveTask(task.id)
                                break
                            }
                        }
                    } else {
                        Log.e(TAG, "Failed to check status: ${parsedStatus["error"]}")
                        break
                    }
                }
            } catch (e: CancellationException) {
                Log.d(TAG, "Monitoring cancelled for ${task.id}")
            } catch (e: Exception) {
                Log.e(TAG, "Error monitoring download ${task.id}", e)
            } finally {
                monitoringJobs.remove(task.id)
            }
        }

        monitoringJobs[task.id] = job
    }

    /**
     * Complete a plain MP3 download without FFmpeg decryption.
     */
    private suspend fun completePlainAudioDownload(
        task: Task,
        downloadPath: String,
        outputDirectory: String
    ) = withContext(Dispatchers.IO) {
        val asin = task.getMetadataString(DownloadTaskMetadata.ASIN) ?: return@withContext
        val title = task.getMetadataString(DownloadTaskMetadata.TITLE) ?: return@withContext

        try {
            manager.updateTaskMetadata(task.id, mapOf(
                DownloadTaskMetadata.STAGE to "copying",
                DownloadTaskMetadata.PERCENTAGE to 0,
                DownloadTaskMetadata.ETA_SECONDS to 0
            ))
            task.getMetadataString(DownloadTaskMetadata.RUST_TASK_ID)?.let { updateRustTaskStatusInDb(it, "copying") }
            manager.emitEvent(TaskEvent.DownloadProgress(
                taskId = task.id,
                asin = asin,
                title = title,
                stage = "copying",
                percentage = 0,
                bytesDownloaded = 0,
                totalBytes = 0
            ))

            val finalPath = copyToFinalDestination(asin, title, downloadPath, outputDirectory, null, task.id)

            task.getMetadataString(DownloadTaskMetadata.RUST_TASK_ID)?.let { rustTaskId ->
                updateRustTaskStatusInDb(rustTaskId, "completed", finalPath)
            }
            task.status = TaskStatus.COMPLETED
            task.completedAt = java.util.Date()
            manager.emitEvent(TaskEvent.DownloadComplete(
                taskId = task.id,
                asin = asin,
                title = title,
                outputPath = finalPath
            ))
            manager.emitEvent(TaskEvent.TaskCompleted(task))
            manager.unregisterActiveTask(task.id)

            clearManuallyPaused(asin)

            Log.d(TAG, "Plain audio download complete: $asin")
        } catch (e: Exception) {
            Log.e(TAG, "Plain audio copy failed for $asin", e)
            task.status = TaskStatus.FAILED
            task.error = e.message
            task.completedAt = java.util.Date()
            manager.emitEvent(TaskEvent.TaskFailed(task, e.message ?: "Copy failed"))
            manager.unregisterActiveTask(task.id)
        }
    }

    /**
     * Trigger conversion after download completes
     */
    private suspend fun triggerConversion(
        task: Task,
        encryptedPath: String,
        decryptedCachePath: String,
        outputDirectory: String,
        aaxcKey: String,
        aaxcIv: String
    ) = withContext(Dispatchers.IO) {
        val asin = task.getMetadataString(DownloadTaskMetadata.ASIN) ?: return@withContext
        val title = task.getMetadataString(DownloadTaskMetadata.TITLE) ?: return@withContext

        // Clear any stale cancel flag from a previous attempt for this book.
        cancelledConversions.remove(asin)

        try {
            Log.d(TAG, "Starting conversion for $asin...")

            // Update stage
            manager.updateTaskMetadata(task.id, mapOf(
                DownloadTaskMetadata.STAGE to "decrypting",
                DownloadTaskMetadata.PERCENTAGE to 0,
                DownloadTaskMetadata.ETA_SECONDS to 0
            ))
            // Persist the stage to the download-task DB row so the library list reflects it.
            task.getMetadataString(DownloadTaskMetadata.RUST_TASK_ID)?.let { updateRustTaskStatusInDb(it, "decrypting") }
            manager.emitEvent(TaskEvent.DownloadProgress(
                taskId = task.id,
                asin = asin,
                title = title,
                stage = "decrypting",
                percentage = 0,
                bytesDownloaded = 0,
                totalBytes = 0
            ))

            // Fetch metadata from database
            val metadata = fetchBookMetadata(asin)
            Log.d(TAG, "Fetched metadata for $asin: ${metadata?.keys?.joinToString(", ") ?: "null"}")
            if (metadata != null) {
                Log.d(TAG, "  title: ${metadata["title"]}")
                Log.d(TAG, "  authors: ${metadata["authors"]} (${metadata["authors"]?.javaClass?.simpleName})")
                Log.d(TAG, "  narrators: ${metadata["narrators"]} (${metadata["narrators"]?.javaClass?.simpleName})")
                Log.d(TAG, "  publisher: ${metadata["publisher"]}")
                Log.d(TAG, "  series_name: ${metadata["series_name"]}")
            }

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

            Log.d(TAG, "FFmpeg command for $asin: $command")

            // Probe total duration (encrypted container metadata is cleartext) so decrypt can
            // report live progress + ETA into task metadata; the notification poll renders it.
            val totalDurationSec = com.arthenica.ffmpegkit.FFprobeKit
                .getMediaInformation(encryptedPath)
                .mediaInformation?.duration?.toDoubleOrNull() ?: 0.0

            var lastReportedPct = -1
            val statsCallback = com.arthenica.ffmpegkit.StatisticsCallback { stat ->
                if (totalDurationSec > 0.0) {
                    val processedSec = stat.time.toDouble() / 1000.0
                    val pct = ((processedSec / totalDurationSec).coerceIn(0.0, 1.0) * 100.0).toInt()
                    if (pct > lastReportedPct) {
                        lastReportedPct = pct
                        val speed = stat.speed
                        val etaSec = if (speed > 0.0)
                            ((totalDurationSec - processedSec) / speed).toInt().coerceAtLeast(0)
                        else 0
                        manager.updateTaskMetadata(task.id, mapOf(
                            DownloadTaskMetadata.PERCENTAGE to pct,
                            DownloadTaskMetadata.ETA_SECONDS to etaSec
                        ))
                    }
                }
            }

            // executeAsync exposes the session id up front so this specific decrypt can be
            // cancelled; a blocking execute() would give no handle to cancel just this one.
            val ffmpegLatch = java.util.concurrent.CountDownLatch(1)
            val session = com.arthenica.ffmpegkit.FFmpegKit.executeAsync(
                command,
                { _ -> ffmpegLatch.countDown() },
                { _ -> },
                statsCallback
            )
            activeFfmpegSessions[asin] = session.sessionId
            try {
                ffmpegLatch.await()
            } finally {
                activeFfmpegSessions.remove(asin)
            }

            // Cleanup cover art temp file
            coverArtPath?.let { File(it).delete() }

            if (asin in cancelledConversions) {
                throw kotlinx.coroutines.CancellationException("Decrypt cancelled by user")
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
            manager.updateTaskMetadata(task.id, mapOf(
                DownloadTaskMetadata.STAGE to "validating",
                DownloadTaskMetadata.PERCENTAGE to 0,
                DownloadTaskMetadata.ETA_SECONDS to 0
            ))
            task.getMetadataString(DownloadTaskMetadata.RUST_TASK_ID)?.let { updateRustTaskStatusInDb(it, "validating") }
            manager.emitEvent(TaskEvent.DownloadProgress(
                taskId = task.id,
                asin = asin,
                title = title,
                stage = "validating",
                percentage = 0,
                bytesDownloaded = 0,
                totalBytes = 0
            ))

            val validationResult = validateAudioFile(
                context,
                decryptedCachePath,
                isCancelled = { asin in cancelledConversions }
            ) { pct, eta ->
                manager.updateTaskMetadata(task.id, mapOf(
                    DownloadTaskMetadata.PERCENTAGE to pct,
                    DownloadTaskMetadata.ETA_SECONDS to eta
                ))
            }

            if (!validationResult.isValid) {
                Log.e(TAG, "Audio validation FAILED for $asin:")
                Log.e(TAG, "  Error count: ${validationResult.errorCount}")
                Log.e(TAG, "  Duration: ${validationResult.duration}s")
                Log.e(TAG, "  Message: ${validationResult.errorMessage}")

                // Delete corrupt file
                File(decryptedCachePath).delete()
                File(encryptedPath).delete()

                throw Exception("Audio file validation failed: Corruption detected at multiple points. ${validationResult.errorMessage}")
            }

            Log.d(TAG, "✓ Audio validation PASSED for $asin (${validationResult.duration}s, 0 errors)")

            // Update stage
            manager.updateTaskMetadata(task.id, mapOf(
                DownloadTaskMetadata.STAGE to "copying",
                DownloadTaskMetadata.PERCENTAGE to 0,
                DownloadTaskMetadata.ETA_SECONDS to 0
            ))
            task.getMetadataString(DownloadTaskMetadata.RUST_TASK_ID)?.let { updateRustTaskStatusInDb(it, "copying") }
            manager.emitEvent(TaskEvent.DownloadProgress(
                taskId = task.id,
                asin = asin,
                title = title,
                stage = "copying",
                percentage = 0,
                bytesDownloaded = 0,
                totalBytes = 0
            ))

            // Copy to final destination
            val finalPath = copyToFinalDestination(asin, title, decryptedCachePath, outputDirectory, coverArtPath, task.id)

            // Cleanup encrypted file
            File(encryptedPath).delete()

            // Cleanup cover art temp file
            coverArtPath?.let { File(it).delete() }

            // Mark as completed
            task.getMetadataString(DownloadTaskMetadata.RUST_TASK_ID)?.let { rustTaskId ->
                updateRustTaskStatusInDb(rustTaskId, "completed", finalPath)
            }
            task.status = TaskStatus.COMPLETED
            task.completedAt = java.util.Date()
            manager.emitEvent(TaskEvent.DownloadComplete(
                taskId = task.id,
                asin = asin,
                title = title,
                outputPath = finalPath
            ))
            manager.emitEvent(TaskEvent.TaskCompleted(task))
            manager.unregisterActiveTask(task.id)

            // Clear manual pause marker
            clearManuallyPaused(asin)

            Log.d(TAG, "Download task complete: $asin")

        } catch (e: Exception) {
            if (e is kotlinx.coroutines.CancellationException || asin in cancelledConversions) {
                // User cancelled mid-conversion: drop the partial cache file, mark cancelled,
                // no error notification.
                Log.d(TAG, "Conversion cancelled for $asin")
                runCatching { File(decryptedCachePath).delete() }
                task.getMetadataString(DownloadTaskMetadata.RUST_TASK_ID)?.let { updateRustTaskStatusInDb(it, "cancelled") }
                task.status = TaskStatus.CANCELLED
                task.completedAt = java.util.Date()
                manager.emitEvent(TaskEvent.TaskCancelled(task))
                manager.unregisterActiveTask(task.id)
            } else {
                Log.e(TAG, "Conversion failed for $asin", e)
                task.status = TaskStatus.FAILED
                task.error = e.message
                task.completedAt = java.util.Date()
                manager.emitEvent(TaskEvent.TaskFailed(task, e.message ?: "Conversion failed"))
                manager.unregisterActiveTask(task.id)
            }
        } finally {
            cancelledConversions.remove(asin)
            activeFfmpegSessions.remove(asin)
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
        coverArtPath: String?,
        taskId: String
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

            // Navigate/create subdirectories (tracking newly-created ones so a cancelled copy
            // can remove the empty folders it made).
            var currentDir = docDir
            val createdDirs = mutableListOf<DocumentFile>()
            for (dirName in directories) {
                val existing = currentDir.findFile(dirName)
                currentDir = if (existing != null && existing.isDirectory) {
                    existing
                } else {
                    currentDir.createDirectory(dirName)
                        ?.also { createdDirs.add(it) }
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

            // Copy (with progress + ETA — large M4B over SAF is slow). A cancel mid-copy throws
            // CancellationException from copyStreamWithProgress; delete the partial output and any
            // folders we just created so a cancelled download leaves nothing behind.
            val totalBytes = cachedFile.length()
            try {
                context.contentResolver.openOutputStream(outputFile.uri)?.use { outputStream ->
                    cachedFile.inputStream().use { inputStream ->
                        copyStreamWithProgress(inputStream, outputStream, totalBytes, { asin in cancelledConversions }) { pct, eta ->
                            manager.updateTaskMetadata(taskId, mapOf(
                                DownloadTaskMetadata.PERCENTAGE to pct,
                                DownloadTaskMetadata.ETA_SECONDS to eta
                            ))
                        }
                    }
                } ?: throw Exception("Failed to open output stream")
            } catch (e: Exception) {
                runCatching { outputFile.delete() }
                createdDirs.asReversed().forEach { dir ->
                    runCatching { if (dir.listFiles().isEmpty()) dir.delete() }
                }
                throw e
            }

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
     * Pause a download
     */
    suspend fun pause(taskId: String): Boolean = withContext(Dispatchers.IO) {
        try {
            val task = manager.getTask(taskId) ?: return@withContext false
            val rustTaskId = task.getMetadataString(DownloadTaskMetadata.RUST_TASK_ID) ?: return@withContext false
            val asin = task.getMetadataString(DownloadTaskMetadata.ASIN) ?: return@withContext false

            val pauseParams = JSONObject().apply {
                put("db_path", manager.getDbPath())
                put("task_id", rustTaskId)
            }

            val result = ExpoRustBridgeModule.nativePauseDownload(pauseParams.toString())
            val parsed = parseJsonResponse(result)

            if (parsed["success"] == true) {
                markAsManuallyPaused(asin)
                task.status = TaskStatus.PAUSED
                manager.emitEvent(TaskEvent.TaskPaused(task))
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
     * Resume a download
     */
    suspend fun resume(taskId: String): Boolean = withContext(Dispatchers.IO) {
        try {
            val task = manager.getTask(taskId) ?: return@withContext false
            val rustTaskId = task.getMetadataString(DownloadTaskMetadata.RUST_TASK_ID) ?: return@withContext false
            val asin = task.getMetadataString(DownloadTaskMetadata.ASIN) ?: return@withContext false

            val resumeParams = JSONObject().apply {
                put("db_path", manager.getDbPath())
                put("task_id", rustTaskId)
            }

            val result = ExpoRustBridgeModule.nativeResumeDownload(resumeParams.toString())
            val parsed = parseJsonResponse(result)

            if (parsed["success"] == true) {
                clearManuallyPaused(asin)
                task.status = TaskStatus.RUNNING
                manager.emitEvent(TaskEvent.TaskResumed(task))
                Log.d(TAG, "Manually resumed download: $asin")
                true
            } else {
                val error = parsed["error"] as? String
                Log.e(TAG, "Failed to resume: $error")

                // If task is already completed/cancelled in Rust, clean it up from manager
                if (error?.contains("Completed") == true || error?.contains("Cancelled") == true) {
                    Log.d(TAG, "Task already finished in Rust, removing from manager: $asin")
                    task.status = TaskStatus.COMPLETED
                    task.completedAt = java.util.Date()
                    manager.unregisterActiveTask(taskId)
                    clearManuallyPaused(asin)
                }

                false
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error resuming download", e)
            false
        }
    }

    /**
     * Cancel a download
     */
    suspend fun cancel(taskId: String): Boolean = withContext(Dispatchers.IO) {
        try {
            val task = manager.getTask(taskId) ?: return@withContext false
            val asin = task.getMetadataString(DownloadTaskMetadata.ASIN)
            val rustTaskId = task.getMetadataString(DownloadTaskMetadata.RUST_TASK_ID)

            // Abort an in-flight decrypt/validate/copy for this book (no-op if it's still
            // downloading). Without this, cancelling past the download stage would leave the
            // FFmpeg conversion / SAF copy running and the task stuck.
            asin?.let { abortConversion(it) }

            // Stop monitoring
            monitoringJobs[taskId]?.cancel()
            monitoringJobs.remove(taskId)

            // Best-effort cancel of the Rust download — only meaningful while still downloading;
            // past that the download row is already complete and there is nothing to cancel.
            // Log a refusal so a download that keeps running unmonitored is diagnosable.
            if (rustTaskId != null) {
                val cancelParams = JSONObject().apply {
                    put("db_path", manager.getDbPath())
                    put("task_id", rustTaskId)
                }
                runCatching { parseJsonResponse(ExpoRustBridgeModule.nativeCancelDownload(cancelParams.toString())) }
                    .onSuccess { if (it["success"] != true) Log.w(TAG, "nativeCancelDownload refused for $asin: ${it["error"]}") }
                    .onFailure { Log.w(TAG, "nativeCancelDownload failed for $asin", it) }
            }

            // Mark cancelled regardless of the native result: the conversion abort above covers
            // the post-download stages, and the previous success-gated path left those stuck.
            asin?.let { clearManuallyPaused(it) }
            task.status = TaskStatus.CANCELLED
            task.completedAt = java.util.Date()
            manager.emitEvent(TaskEvent.TaskCancelled(task))
            manager.unregisterActiveTask(taskId)
            Log.d(TAG, "Cancelled download: ${asin ?: taskId}")
            true
        } catch (e: Exception) {
            Log.e(TAG, "Error cancelling download", e)
            false
        }
    }

    /**
     * Abort an in-flight conversion (decrypt/validate/copy) for [asin]: set the cancel flag the
     * pipeline checks and kill just this book's FFmpeg session (not every parallel one).
     */
    fun abortConversion(asin: String) {
        cancelledConversions.add(asin)
        activeFfmpegSessions[asin]?.let {
            com.arthenica.ffmpegkit.FFmpegKit.cancel(it)
        }
    }

    /**
     * Stop every monitoring loop and abort every in-flight conversion. Used by
     * clearAllTasks for stuck-state recovery so a cleared task stops polling/logging instead
     * of looping forever (the "residual undeletable task" symptom).
     */
    fun cancelAllMonitoring() {
        // Flag every monitored book as cancelled, not just those with a live FFmpeg session:
        // a book in the validating or copying stage has no session, and Job.cancel() alone
        // cannot interrupt those loops — only the cancelledConversions flag stops them.
        monitoringJobs.keys.toList().forEach { taskId ->
            manager.getTask(taskId)?.getMetadataString(DownloadTaskMetadata.ASIN)
                ?.let { cancelledConversions.add(it) }
        }
        monitoringJobs.values.forEach { runCatching { it.cancel() } }
        monitoringJobs.clear()
        activeFfmpegSessions.keys.toList().forEach { asin -> abortConversion(asin) }
    }

    /**
     * Restore pending download tasks from database
     */
    suspend fun restorePendingTasks() = withContext(Dispatchers.IO) {
        try {
            val listParams = JSONObject().apply {
                put("db_path", manager.getDbPath())
            }

            val listResult = ExpoRustBridgeModule.nativeListDownloadTasks(listParams.toString())
            val parsed = parseJsonResponse(listResult)

            if (parsed["success"] == true) {
                val data = parsed["data"] as? Map<*, *>
                @Suppress("UNCHECKED_CAST")
                val tasks = data?.get("tasks") as? List<Map<*, *>> ?: emptyList()

                tasks.forEach { rustTask ->
                    val status = rustTask["status"] as? String
                    if (status in listOf("queued", "downloading", "paused")) {
                        Log.d(TAG, "Found pending download: ${rustTask["asin"]} (status: $status)")
                        // TODO: Restore monitoring for these tasks
                    }
                }
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error restoring pending tasks", e)
        }
    }

    // ========================================================================
    // Manual Pause Tracking
    // ========================================================================

    private fun markAsManuallyPaused(asin: String) {
        val manuallyPaused = getManuallyPausedAsins().toMutableSet()
        manuallyPaused.add(asin)
        prefs.edit().putStringSet(PREF_MANUALLY_PAUSED, manuallyPaused).apply()
        Log.d(TAG, "Marked $asin as manually paused")
    }

    private fun clearManuallyPaused(asin: String) {
        val manuallyPaused = getManuallyPausedAsins().toMutableSet()
        if (manuallyPaused.remove(asin)) {
            prefs.edit().putStringSet(PREF_MANUALLY_PAUSED, manuallyPaused).apply()
            Log.d(TAG, "Cleared manual pause marker for $asin")
        }
    }

    private fun getManuallyPausedAsins(): Set<String> {
        return prefs.getStringSet(PREF_MANUALLY_PAUSED, emptySet()) ?: emptySet()
    }

    /**
     * Keep the persistent Rust download task in sync with the final converted file path.
     */
    private fun updateRustTaskStatusInDb(taskId: String, status: String, outputPath: String? = null) {
        try {
            val params = JSONObject().apply {
                put("db_path", manager.getDbPath())
                put("task_id", taskId)
                put("status", status)
                outputPath?.let { put("output_path", it) }
            }
            ExpoRustBridgeModule.nativeUpdateDownloadTaskStatus(params.toString())
            Log.d(TAG, "Updated Rust task $taskId status to $status")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to update Rust task status: ${e.message}")
        }
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
                put("db_path", manager.getDbPath())
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
                put("db_path", manager.getDbPath())
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
