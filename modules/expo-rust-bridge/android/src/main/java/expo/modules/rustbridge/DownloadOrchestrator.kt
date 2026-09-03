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
        // Output format, shared by every provider. "m4b" = one file per book,
        // "mp3" = one MP3 per chapter/part. Stored by the Settings screen.
        private const val FORMAT_M4B = "m4b"
        private const val FORMAT_MP3 = "mp3"
        // Audible refuses licences for a while once it decides an account is asking too
        // often ("CustomerThrottled"). Retrying into that refusal is what prolongs it, so
        // record the refusal and stop asking until this much time has passed.
        private const val PREF_LICENSE_THROTTLED_UNTIL = "license_throttled_until"
        private const val LICENSE_THROTTLE_COOLDOWN_MS = 60 * 60 * 1000L
    }

    private val prefs: SharedPreferences = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())

    // Network monitoring
    private val connectivityManager = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    private var isWifiAvailable = false

    // Active download monitoring jobs
    private val monitoringJobs = mutableMapOf<String, Job>()
    // Cancellation of an in-flight conversion (decrypt / validate / copy), keyed by ASIN.
    private val cancelledConversions = java.util.concurrent.ConcurrentHashMap.newKeySet<String>()
    // Running FFmpeg session id per ASIN, so a specific decrypt can be cancelled without
    // FFmpegKit.cancel() (which would kill every parallel conversion).
    private val activeFfmpegSessions = java.util.concurrent.ConcurrentHashMap<String, Long>()
    // Chapter markers from the provider's plan, kept until the book is finalized so the
    // per-chapter MP3 step knows where to cut. Empty for providers that supply none; the
    // encoder then falls back to the chapters embedded in the decrypted file.
    private val planChapters = java.util.concurrent.ConcurrentHashMap<String, List<PlanChapter>>()

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
     * Execute a typed [DownloadPlan] — the single, provider-agnostic download entry point.
     *
     * Enqueues the Rust download task for the plan's part and starts the shared monitoring
     * loop; the part's kind decides the finalize step (AAXC decrypt vs plain/zip copy), which
     * [startMonitoringDownload] already dispatches via its `plainAudio` flag. Audible and
     * LibriVox both build a single-part plan and route here; Libro.fm (Phase 4) supplies its
     * plan from the Rust provider. Multi-part plans (Libro.fm parts-folder) are added in Phase 4.
     */
    suspend fun enqueuePlan(
        asin: String,
        title: String,
        plan: DownloadPlan,
        outputDirectory: String,
    ): String {
        if (plan.parts.isEmpty()) throw Exception("Empty download plan for $asin")
        if (plan.chapters.isNotEmpty()) planChapters[asin] = plan.chapters
        return enqueuePlanPart(asin, title, plan, 0, outputDirectory)
    }

    /**
     * Enqueue part [index] of a plan. Multi-part plans (Libro.fm parts-folder) run
     * **sequentially**: each part chains the next once it has been extracted into the
     * book folder, so one book stays one in-flight download from the queue's, the
     * notification's and the user's point of view.
     */
    private suspend fun enqueuePlanPart(
        asin: String,
        title: String,
        plan: DownloadPlan,
        index: Int,
        outputDirectory: String,
    ): String = withContext(Dispatchers.IO) {
        val part = plan.parts[index]
        val isLastPart = index == plan.parts.lastIndex
        // Distinct cache names so a paused/failed part can't collide with its siblings.
        val partPrefix = if (plan.parts.size > 1) "part${index + 1}-" else ""

        val audiobooksDir = File(context.cacheDir, "audiobooks").apply { mkdirs() }

        // Resolve cache paths + finalize mode from the part kind.
        val encryptedPath: String
        val decryptedCachePath: String
        val plainAudio: Boolean
        val aaxcKey: String
        val aaxcIv: String
        when (part) {
            is DownloadPart.AaxcPart -> {
                encryptedPath = File(audiobooksDir, part.filename).absolutePath
                decryptedCachePath = File(audiobooksDir, "$asin.m4b").absolutePath
                plainAudio = false
                aaxcKey = part.key
                aaxcIv = part.iv
            }
            is DownloadPart.PlainPart -> {
                encryptedPath = File(audiobooksDir, "$partPrefix${part.filename}").absolutePath
                decryptedCachePath = encryptedPath
                plainAudio = true
                aaxcKey = ""
                aaxcIv = ""
            }
            is DownloadPart.ZipPart -> {
                // Cache with a .zip name so copyPlainAudioToFinalDestination extracts it.
                encryptedPath = File(audiobooksDir, "$partPrefix$asin.zip").absolutePath
                decryptedCachePath = encryptedPath
                plainAudio = true
                aaxcKey = ""
                aaxcIv = ""
            }
        }

        // Enqueue the download in the Rust manager.
        val enqueueParams = JSONObject().apply {
            put("db_path", dbPath)
            put("asin", asin)
            put("title", title)
            put("download_url", part.url)
            put("total_bytes", 0)
            put("download_path", encryptedPath)
            put("output_path", decryptedCachePath)
            put("request_headers", JSONObject(part.headers))
        }
        val parsedEnqueue = parseJsonResponse(ExpoRustBridgeModule.nativeEnqueueDownload(enqueueParams.toString()))
        if (parsedEnqueue["success"] != true) {
            throw Exception("Failed to enqueue: ${parsedEnqueue["error"]}")
        }
        val taskId = (parsedEnqueue["data"] as? Map<*, *>)?.get("task_id") as? String
            ?: throw Exception("No task ID")

        // Persist keys (AAXC) + output dir for the finalize/retry steps.
        storeConversionKeysInDb(taskId, aaxcKey, aaxcIv, outputDirectory)

        startMonitoringDownload(
            taskId = taskId,
            asin = asin,
            title = title,
            encryptedPath = encryptedPath,
            decryptedCachePath = decryptedCachePath,
            outputDirectory = outputDirectory,
            aaxcKey = aaxcKey,
            aaxcIv = aaxcIv,
            totalBytes = 0,
            plainAudio = plainAudio,
            isFinalPart = isLastPart,
        )

        // Multi-part: wait for this part to land in the book folder, then start the next.
        // Chaining here (rather than from inside the monitoring job) keeps a part from
        // cancelling the very job it is running on.
        if (!isLastPart) {
            monitoringJobs[asin]?.join()
            enqueuePlanPart(asin, title, plan, index + 1, outputDirectory)
        }
        taskId
    }

    /**
     * Enqueue a book from any provider in the Rust registry: ask the provider for its
     * typed plan, then execute it. This is the generic path — a new DRM-free provider
     * needs no Kotlin changes at all, only a `providers/` module on the Rust side.
     *
     * `optionsJson` carries provider-specific download settings (Libro.fm's
     * `{"format":"parts"|"m4b"}`); pass `"{}"` when the provider has none.
     */
    suspend fun enqueueProviderBook(
        provider: String,
        accountJson: String,
        itemRef: String,
        title: String,
        outputDirectory: String,
        optionsJson: String = "{}",
    ): String = withContext(Dispatchers.IO) {
        Log.d(TAG, "Enqueueing $provider book: $itemRef - $title")

        val cooldown = licenseCooldownRemainingMs(accountJson)
        if (cooldown > 0) {
            val message = throttleMessage(cooldown)
            Log.w(TAG, "Skipping $itemRef: $message")
            errorCallback?.invoke(itemRef, title, message)
            throw Exception(message)
        }

        try {
            val params = JSONObject().apply {
                put("provider", provider)
                put("db_path", dbPath)
                put("account_json", accountJson)
                put("item_ref", itemRef)
                put("options", JSONObject(optionsJson))
            }
            // Parse the envelope directly rather than via parseJsonResponse: the plan model
            // consumes JSON, and round-tripping through nested Maps would lose the shape.
            val envelope = JSONObject(ExpoRustBridgeModule.nativeProviderGetDownloadPlan(params.toString()))
            if (!envelope.optBoolean("success")) {
                if (envelope.optString("error_code") == "license_throttled") {
                    recordLicenseThrottled(accountJson)
                }
                throw Exception("Download plan failed: ${envelope.optString("error")}")
            }
            clearLicenseThrottled(accountJson)
            val planJson = envelope.optJSONObject("data")
                ?: throw Exception("No download plan returned for $itemRef")
            enqueuePlan(itemRef, title, DownloadPlan.fromJson(planJson), outputDirectory)
        } catch (e: Exception) {
            Log.e(TAG, "Failed to enqueue $provider book", e)
            errorCallback?.invoke(itemRef, title, e.message ?: "Unknown error")
            throw e
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

        // Don't ask for a licence Audible is currently refusing: auto-download would
        // otherwise retry every book in the library into the same refusal.
        val cooldown = licenseCooldownRemainingMs(accountJson)
        if (cooldown > 0) {
            val message = throttleMessage(cooldown)
            Log.w(TAG, "Skipping $asin: $message")
            errorCallback?.invoke(asin, title, message)
            throw Exception(message)
        }

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
                if (parsedLicense["error_code"] == "license_throttled") {
                    recordLicenseThrottled(accountJson)
                }
                throw Exception("License request failed: ${parsedLicense["error"]}")
            }
            clearLicenseThrottled(accountJson)

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

            // Step 2: Build a single-part plan from the license and route through the
            // unified plan engine (AAXC -> decrypt; plain MP3 -> copy).
            val filename = "$asin.$fileExtension"
            val part = if (isPlainAudio) {
                DownloadPart.PlainPart(downloadUrl, requestHeaders, filename)
            } else {
                DownloadPart.AaxcPart(downloadUrl, requestHeaders, aaxcKey.orEmpty(), aaxcIv.orEmpty(), filename)
            }
            enqueuePlan(asin, title, DownloadPlan(listOf(part), embedMetadata = true), outputDirectory)
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
            // Detect extension from URL - check query params (archive.org uses &file=) and path.
            // The cache filename's extension drives finalize: a .zip is extracted, else copied.
            val parsedUrl = Uri.parse(downloadUrl)
            val fileParam = parsedUrl.getQueryParameter("file")
            val urlForExt = fileParam ?: parsedUrl.lastPathSegment ?: downloadUrl
            val extension = urlForExt.substringAfterLast('.', "mp3").substringBefore('?').lowercase()

            // DRM-free single-part plan (no license, no special headers) → unified engine.
            val part = DownloadPart.PlainPart(downloadUrl, emptyMap(), "$asin.$extension")
            enqueuePlan(asin, title, DownloadPlan(listOf(part)), outputDirectory)
        } catch (e: Exception) {
            Log.e(TAG, "Failed to enqueue LibriVox book", e)
            errorCallback?.invoke(asin, title, e.message ?: "Unknown error")
            throw e
        }
    }

    /**
     * Copy or extract a DRM-free download to the user's SAF directory — the finalize step
     * for [DownloadPart.PlainPart] and [DownloadPart.ZipPart], whatever provider produced it.
     * A zip is extracted into Author/Title/; anything else is copied as a single file.
     */
    private suspend fun copyPlainAudioToFinalDestination(
        asin: String,
        title: String,
        downloadPath: String,
        outputDirectory: String
    ): String = withContext(Dispatchers.IO) {
        val cachedFile = File(downloadPath)

        // Build directory path using naming pattern (Author/Title/)
        val target = resolveBookTarget(asin, outputDirectory)

        val extension = cachedFile.extension.lowercase()

        val finalPath = if (extension == "zip") {
            extractZipToDirectory(cachedFile, target.safDir, target.regularPath, asin)
        } else {
            copySingleFileToDirectory(cachedFile, extension, target.fileName, target.safDir, target.regularPath, asin)
        }

        cachedFile.delete()
        Log.d(TAG, "Saved to: $finalPath")
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

    /** Remaining cooldown in ms after Audible throttled this account's licences, or 0. */
    private fun licenseCooldownRemainingMs(accountJson: String?): Long {
        val until = prefs.getLong(throttleKey(accountJson), 0L)
        val remaining = until - System.currentTimeMillis()
        return if (remaining > 0) remaining else 0L
    }

    /** Record that Audible just throttled this account, starting the cooldown. */
    private fun recordLicenseThrottled(accountJson: String?) {
        val until = System.currentTimeMillis() + LICENSE_THROTTLE_COOLDOWN_MS
        prefs.edit().putLong(throttleKey(accountJson), until).apply()
        Log.w(TAG, "Audible throttled licences; pausing download requests for ${LICENSE_THROTTLE_COOLDOWN_MS / 60000} minutes")
    }

    /** A cleared throttle: the last request went through, so stop holding books back. */
    private fun clearLicenseThrottled(accountJson: String?) {
        if (prefs.contains(throttleKey(accountJson))) {
            prefs.edit().remove(throttleKey(accountJson)).apply()
        }
    }

    /**
     * Throttling is per Audible customer, so key the cooldown by account when the id is
     * available and fall back to a shared key when it isn't.
     */
    private fun throttleKey(accountJson: String?): String {
        val accountId = accountJson?.let {
            runCatching { JSONObject(it).optString("account_id").takeIf(String::isNotBlank) }.getOrNull()
        }
        return if (accountId != null) "${PREF_LICENSE_THROTTLED_UNTIL}_$accountId" else PREF_LICENSE_THROTTLED_UNTIL
    }

    /** The message shown while a cooldown is in effect. */
    private fun throttleMessage(remainingMs: Long): String {
        val minutes = ((remainingMs + 59_999) / 60_000).coerceAtLeast(1)
        return "Audible is throttling downloads for this account. Try again in about $minutes minute${if (minutes == 1L) "" else "s"}."
    }

    /**
     * The folder a book's files belong in, plus the file name the naming pattern chose.
     * SAF and plain-filesystem destinations are resolved the same way; exactly one of
     * [safDir] / [regularPath] is non-null.
     */
    private data class BookTarget(
        val safDir: DocumentFile?,
        val regularPath: String?,
        val fileName: String,
    )

    /**
     * Resolve (and create) the destination folder for [asin] from the naming pattern.
     */
    private fun resolveBookTarget(asin: String, outputDirectory: String): BookTarget {
        val docDir = if (outputDirectory.startsWith("content://")) {
            DocumentFile.fromTreeUri(context, Uri.parse(outputDirectory))
                ?: throw Exception("Invalid SAF URI")
        } else null

        if (docDir != null && !docDir.canWrite()) {
            throw Exception("No write permission for SAF directory")
        }

        val filePath = buildFilePathForBook(asin)
        Log.d(TAG, "Using file path: $filePath")

        val pathParts = filePath.split('/')
        val directories = pathParts.dropLast(1)

        return if (docDir != null) {
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
            BookTarget(currentDir, null, pathParts.last())
        } else {
            val dir = File(outputDirectory, directories.joinToString("/"))
            dir.mkdirs()
            BookTarget(null, dir.absolutePath, pathParts.last())
        }
    }

    /**
     * The user's output format, shared by every provider: one file per book, or one
     * MP3 per chapter/part. `parts` is the pre-0.0.28 Libro.fm-only value for MP3.
     */
    private fun downloadFormat(): String {
        val settings = context.getSharedPreferences("app_settings", Context.MODE_PRIVATE)
        val stored = settings.getString("download_format", null)
            ?: settings.getString("librofm_format", null)
            ?: FORMAT_M4B
        return if (stored == FORMAT_MP3 || stored == "parts") FORMAT_MP3 else FORMAT_M4B
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
                sourceFile.inputStream().use { inp -> copyStreamWithProgress(inp, out, totalBytes, { asin in cancelledConversions }, onCopyProgress) }
            } ?: throw Exception("Failed to write: $targetName")
            outputFile.uri.toString()
        } else {
            val outputFile = File(regularDirPath!!, targetName)
            outputFile.outputStream().use { out ->
                sourceFile.inputStream().use { inp -> copyStreamWithProgress(inp, out, totalBytes, { asin in cancelledConversions }, onCopyProgress) }
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
        plainAudio: Boolean = false,
        isFinalPart: Boolean = true
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
                                        triggerPlainAudioCopy(asin, title, encryptedPath, outputDirectory, taskId, isFinalPart)
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
            } catch (e: CancellationException) {
                // Normal path: the download/conversion was cancelled or the coroutine scope was
                // torn down. Not an error — log quietly and rethrow to respect structured
                // concurrency (previously this was logged at ERROR, spamming the log on cancel).
                Log.d(TAG, "Monitoring cancelled for $asin")
                throw e
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
     * Copy (or extract) a completed DRM-free download to the final destination.
     *
     * A non-final part of a multi-part plan ([isFinalPart] = false) has already put its
     * files in the book folder, but the book isn't done: its task row is left without an
     * output path and no completion is reported, so the library keeps showing the book as
     * in progress until the last part lands.
     */
    private suspend fun triggerPlainAudioCopy(
        asin: String,
        title: String,
        downloadPath: String,
        outputDirectory: String,
        taskId: String,
        isFinalPart: Boolean = true
    ) = withContext(Dispatchers.IO) {
        try {
            updateTaskStatusInDb(taskId, "copying")
            progressCallback?.invoke(asin, "copying", 0.0, 0, 0, 0L)

            val finalPath = copyPlainAudioToFinalDestination(asin, title, downloadPath, outputDirectory)

            // Remove the cache file once copied (matches the old LibriVox copy path).
            File(downloadPath).delete()

            if (!isFinalPart) {
                updateTaskStatusInDb(taskId, "completed")
                return@withContext
            }

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

        // Fresh conversion: clear any stale cancel flag from a previous attempt.
        cancelledConversions.remove(asin)

        try {
            Log.d(TAG, "Starting conversion for $asin...")

            // Guard: a stale "completed" download task (its cache deleted by an earlier failed
            // convert) or a 0-byte download (e.g. an auth-failed response) leaves the encrypted
            // source empty or missing. Decrypting it yields a misleading "activation bytes wrong"
            // error. Detect it up front, reset the task to "cancelled" so the library offers a
            // fresh download, and report an honest message. Audiobooks are always far larger than
            // this floor, so it only trips on empty/truncated files.
            val encFile = File(encryptedPath)
            if (!encFile.exists() || encFile.length() < 64 * 1024L) {
                val len = if (encFile.exists()) encFile.length() else -1L
                Log.e(TAG, "Encrypted source missing/too small for $asin ($len bytes) — download incomplete; resetting for re-download")
                resolvedTaskId?.let { updateTaskStatusInDb(it, "cancelled") }
                errorCallback?.invoke(asin, title, "Download incomplete — please download again")
                return@withContext
            }

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
            val statsCallback = com.arthenica.ffmpegkit.StatisticsCallback { stat ->
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
            resolvedTaskId?.let { updateTaskStatusInDb(it, "validating") }
            progressCallback?.invoke(asin, "validating", 0.0, 0, 0, 0L)

            val validationResult = validateAudioFile(
                context,
                decryptedCachePath,
                isCancelled = { asin in cancelledConversions }
            ) { pct, eta -> progressCallback?.invoke(asin, "validating", pct.toDouble(), 0, 0, eta.toLong()) }

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

            // MP3 output: split the decrypted M4B into one MP3 per chapter rather than
            // saving the single file. Books without usable chapter markers fall back to
            // the single-file copy inside encodeChaptersToMp3.
            val finalPath = if (downloadFormat() == FORMAT_MP3) {
                encodeChaptersToMp3(asin, title, decryptedCachePath, outputDirectory, coverArtPath, resolvedTaskId)
            } else {
                // Notify copying stage
                resolvedTaskId?.let { updateTaskStatusInDb(it, "copying") }
                progressCallback?.invoke(asin, "copying", 0.0, 0, 0, 0L)

                // Copy to final destination
                copyToFinalDestination(asin, title, decryptedCachePath, outputDirectory, coverArtPath)
            }

            // Cleanup encrypted file
            File(encryptedPath).delete()

            // Cleanup cover art temp file
            coverArtPath?.let { File(it).delete() }

            // Mark as completed in DB with the final SAF/file path
            resolvedTaskId?.let { updateTaskStatusInDb(it, "completed", finalPath) }
            planChapters.remove(asin)

        } catch (e: Exception) {
            if (e is kotlinx.coroutines.CancellationException || asin in cancelledConversions) {
                // User cancelled mid-conversion: clean up partial output, no error UI.
                // Rethrow as cancellation so the monitor coroutine ignores it (no error
                // notification); the finally clears the per-book cancel state.
                Log.d(TAG, "Conversion cancelled for $asin")
                runCatching { File(decryptedCachePath).delete() }
                resolvedTaskId?.let { updateTaskStatusInDb(it, "cancelled") }
                throw kotlinx.coroutines.CancellationException("Conversion cancelled")
            }
            Log.e(TAG, "Conversion failed for $asin", e)
            // Mark as failed in DB with error
            resolvedTaskId?.let { updateTaskStatusWithError(it, "failed", e.message ?: "Conversion failed") }
            errorCallback?.invoke(asin, title, e.message ?: "Conversion failed")
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

            // Navigate/create subdirectories, remembering which ones we created so a
            // cancelled copy can remove them (but never a pre-existing folder).
            val createdDirs = mutableListOf<DocumentFile>()
            var currentDir = docDir
            for (dirName in directories) {
                val existing = currentDir.findFile(dirName)
                currentDir = if (existing != null && existing.isDirectory) {
                    existing
                } else {
                    (currentDir.createDirectory(dirName)
                        ?: throw Exception("Failed to create directory: $dirName"))
                        .also { createdDirs.add(it) }
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
            try {
                context.contentResolver.openOutputStream(outputFile.uri)?.use { outputStream ->
                    cachedFile.inputStream().use { inputStream ->
                        copyStreamWithProgress(inputStream, outputStream, totalBytes, { asin in cancelledConversions }) { pct, eta ->
                            progressCallback?.invoke(asin, "copying", pct.toDouble(), 0, 0, eta.toLong())
                        }
                    }
                } ?: throw Exception("Failed to open output stream")
            } catch (e: Exception) {
                // Cancel or failure mid-copy: remove the partial file and any folders we
                // created for it, so it isn't found + relinked by a later library scan.
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

        // Clear manual pause marker on completion
        clearManuallyPaused(asin)

        completionCallback?.invoke(asin, title, finalPath)

        finalPath
    }

    /** What a probe of the decrypted file tells us: its own chapters, and its bitrate. */
    private data class SourceInfo(val chapters: List<PlanChapter>, val bitrateK: Int)

    /**
     * Probe the decrypted file for embedded chapters and audio bitrate. Both are
     * best-effort: chapters are only a fallback for providers that supply none, and an
     * unreadable bitrate just means encoding at the spoken-word default.
     */
    private fun probeSource(path: String): SourceInfo {
        return try {
            val session = com.arthenica.ffmpegkit.FFprobeKit.executeWithArguments(
                arrayOf(
                    "-v", "quiet", "-print_format", "json",
                    "-show_chapters", "-show_streams", "-show_format", "-i", path
                )
            )
            val json = JSONObject(session.output ?: "{}")

            val chapterArray = json.optJSONArray("chapters") ?: org.json.JSONArray()
            val chapters = ArrayList<PlanChapter>(chapterArray.length())
            for (i in 0 until chapterArray.length()) {
                val c = chapterArray.optJSONObject(i) ?: continue
                val start = (c.optString("start_time").toDoubleOrNull() ?: continue) * 1000.0
                val end = (c.optString("end_time").toDoubleOrNull() ?: continue) * 1000.0
                if (end <= start) continue
                val chapterTitle = c.optJSONObject("tags")?.optString("title").orEmpty()
                chapters.add(PlanChapter(chapterTitle, start.toLong(), end.toLong()))
            }

            // Prefer the audio stream's bitrate; some containers only report it per-file.
            val streams = json.optJSONArray("streams") ?: org.json.JSONArray()
            var bitrate: Long? = null
            for (i in 0 until streams.length()) {
                val stream = streams.optJSONObject(i) ?: continue
                if (stream.optString("codec_type") == "audio") {
                    bitrate = stream.optString("bit_rate").toLongOrNull()
                    break
                }
            }
            if (bitrate == null) {
                bitrate = json.optJSONObject("format")?.optString("bit_rate")?.toLongOrNull()
            }
            // Audiobooks are typically 64 kbps mono; keep the source rate but never encode
            // absurdly high (pointless re-encode bloat) or low (audible damage).
            val bitrateK = ((bitrate ?: 0L) / 1000L).toInt().coerceIn(48, 128)

            SourceInfo(chapters, bitrateK)
        } catch (e: Exception) {
            Log.w(TAG, "Probe failed for $path: ${e.message}")
            SourceInfo(emptyList(), 64)
        }
    }

    /** Strip characters that are illegal (or merely painful) in file names. */
    private fun sanitizeFileName(name: String): String {
        val cleaned = name.replace(Regex("[/\\\\:*?\"<>|\\x00-\\x1f]"), " ")
            .replace(Regex("\\s+"), " ")
            .trim()
            .trimEnd('.')
        return if (cleaned.isEmpty()) "Chapter" else cleaned.take(80)
    }

    /** Delete a file we wrote, whether it landed in SAF or on the plain filesystem. */
    private fun deleteWrittenFile(path: String) {
        if (path.startsWith("content://")) {
            DocumentFile.fromSingleUri(context, Uri.parse(path))?.delete()
        } else {
            File(path).delete()
        }
    }

    /**
     * Encode one chapter of [source] to [output] as MP3.
     *
     * Registers the FFmpeg session so a cancel from the notification stops this chapter
     * rather than every parallel conversion.
     */
    private fun encodeChapter(
        asin: String,
        source: File,
        output: File,
        chapter: PlanChapter,
        coverArtPath: String?,
        bitrateK: Int,
        chapterTitle: String,
        album: String,
        artist: String?,
        number: Int,
        total: Int,
    ) {
        output.delete()
        val startSec = chapter.startMs / 1000.0
        val durationSec = (chapter.endMs - chapter.startMs) / 1000.0

        // executeWithArguments (not a joined command string): chapter titles and folder
        // names contain spaces and quotes, and an argument array needs no escaping.
        val args = buildList {
            add("-nostdin")
            add("-y")
            add("-ss")
            add(String.format(java.util.Locale.US, "%.3f", startSec))
            add("-i")
            add(source.absolutePath)
            if (coverArtPath != null) {
                add("-i")
                add(coverArtPath)
            }
            add("-t")
            add(String.format(java.util.Locale.US, "%.3f", durationSec))
            add("-map")
            add("0:a:0")
            if (coverArtPath != null) {
                add("-map")
                add("1:v:0")
                add("-c:v")
                add("mjpeg")
                add("-disposition:v:0")
                add("attached_pic")
            }
            add("-c:a")
            add("libmp3lame")
            add("-b:a")
            add("${bitrateK}k")
            add("-id3v2_version")
            add("3")
            add("-metadata")
            add("title=$chapterTitle")
            add("-metadata")
            add("album=$album")
            if (artist != null) {
                add("-metadata")
                add("artist=$artist")
                add("-metadata")
                add("album_artist=$artist")
            }
            add("-metadata")
            add("track=$number/$total")
            add("-metadata")
            add("genre=Audiobook")
            add(output.absolutePath)
        }.toTypedArray()

        val latch = java.util.concurrent.CountDownLatch(1)
        val session = com.arthenica.ffmpegkit.FFmpegKit.executeWithArgumentsAsync(
            args,
            { _ -> latch.countDown() },
            { _ -> },
            { _ -> }
        )
        activeFfmpegSessions[asin] = session.sessionId
        try {
            latch.await()
        } finally {
            activeFfmpegSessions.remove(asin)
        }

        if (asin in cancelledConversions) {
            output.delete()
            throw kotlinx.coroutines.CancellationException("Encode cancelled by user")
        }

        if (!com.arthenica.ffmpegkit.ReturnCode.isSuccess(session.returnCode)) {
            val logs = session.allLogsAsString
            Log.e(TAG, "Chapter $number/$total encode failed for $asin: $logs")
            output.delete()
            throw Exception(ffmpegFailureMessage(logs))
        }
    }

    /**
     * Split the decrypted book into one MP3 per chapter and save them in the book folder.
     *
     * Chapter markers come from the provider's plan (Audible's license `chapter_info`);
     * if the provider supplied none, the file's own markers are probed. A book with
     * neither is saved as a single file rather than as one giant "chapter".
     *
     * Returns the first file written, which becomes the task's output path.
     */
    private suspend fun encodeChaptersToMp3(
        asin: String,
        title: String,
        decryptedCachePath: String,
        outputDirectory: String,
        coverArtPath: String?,
        taskId: String?,
    ): String = withContext(Dispatchers.IO) {
        val source = File(decryptedCachePath)
        val probe = probeSource(decryptedCachePath)
        val fromPlan = planChapters[asin]?.takeIf { it.isNotEmpty() }
        val chapters = fromPlan ?: probe.chapters
        val chapterSource = if (fromPlan != null) "provider plan" else "embedded markers"

        if (chapters.isEmpty()) {
            Log.w(TAG, "No chapter markers for $asin — saving as a single file instead")
            taskId?.let { updateTaskStatusInDb(it, "copying") }
            progressCallback?.invoke(asin, "copying", 0.0, 0, 0, 0L)
            return@withContext copyToFinalDestination(
                asin, title, decryptedCachePath, outputDirectory, coverArtPath
            )
        }

        Log.d(TAG, "Encoding $asin as ${chapters.size} MP3 chapters at ${probe.bitrateK} kbps (chapters from $chapterSource)")
        taskId?.let { updateTaskStatusInDb(it, "encoding") }
        progressCallback?.invoke(asin, "encoding", 0.0, 0, 0, 0L)

        val target = resolveBookTarget(asin, outputDirectory)
        val metadata = fetchBookMetadata(asin)
        val album = metadata?.get("title")?.toString() ?: title
        val artist = metadata?.get("authors")?.toString()
        val workDir = File(context.cacheDir, "audiobooks").apply { mkdirs() }
        val written = mutableListOf<String>()

        try {
            chapters.forEachIndexed { index, chapter ->
                if (asin in cancelledConversions) {
                    throw kotlinx.coroutines.CancellationException("Encode cancelled by user")
                }

                val number = index + 1
                val chapterTitle = chapter.title.ifBlank { "Chapter $number" }
                // Encode to a space-free cache name; the display name is applied on copy.
                val cacheFile = File(workDir, "$asin-chapter-$number.mp3")

                encodeChapter(
                    asin, source, cacheFile, chapter, coverArtPath, probe.bitrateK,
                    chapterTitle, album, artist, number, chapters.size
                )

                val fileName = String.format(
                    java.util.Locale.US, "%02d - %s.mp3", number, sanitizeFileName(chapterTitle)
                )
                written.add(
                    copySingleFileToDirectory(
                        cacheFile, "mp3", fileName, target.safDir, target.regularPath, asin
                    )
                )
                cacheFile.delete()

                progressCallback?.invoke(
                    asin, "encoding", number * 100.0 / chapters.size, 0, 0, 0L
                )
            }
        } catch (e: Exception) {
            // Remove the half-written set: a partial book must not be left behind for the
            // existing-download scanner to link as if it were complete.
            written.forEach { path -> runCatching { deleteWrittenFile(path) } }
            throw e
        }

        source.delete()

        // Smart Audiobook Player cover, same as the single-file path.
        if (coverArtPath != null && target.safDir != null) {
            try {
                val settings = context.getSharedPreferences("app_settings", Context.MODE_PRIVATE)
                if (settings.getString("smart_player_cover_enabled", "false") == "true") {
                    saveSmartPlayerCover(coverArtPath, target.safDir)
                }
            } catch (e: Exception) {
                Log.w(TAG, "Failed to save Smart Audiobook Player cover: ${e.message}")
            }
        }

        Log.d(TAG, "Wrote ${written.size} MP3 chapters for $asin")
        clearManuallyPaused(asin)
        completionCallback?.invoke(asin, title, written.first())
        written.first()
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
     * Abort an in-flight conversion (decrypt / validate / copy) for a book. Marks it
     * cancelled (checked by the copy loop and between validation samples) and cancels
     * its running FFmpeg session so a long decrypt stops promptly. Safe to call when no
     * conversion is running.
     */
    fun abortConversion(asin: String) {
        cancelledConversions.add(asin)
        activeFfmpegSessions[asin]?.let {
            com.arthenica.ffmpegkit.FFmpegKit.cancel(it)
            Log.d(TAG, "Cancelled FFmpeg session $it for $asin")
        }
    }

    /**
     * Abort every in-flight conversion and stop every monitoring loop. Used by the master
     * "stop all" control. The Rust download tasks are cancelled separately (by the caller) so
     * this only tears down the Kotlin-side work.
     */
    fun cancelAll() {
        val asins = monitoringJobs.keys.toList()
        Log.d(TAG, "cancelAll: aborting ${asins.size} monitored downloads/conversions")
        asins.forEach { abortConversion(it) }
        monitoringJobs.values.forEach { runCatching { it.cancel() } }
        monitoringJobs.clear()
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

            // Check if the encrypted file still exists. The extension varies — AAXC
            // downloads are ".aaxc", legacy AAX is ".aax" — so accept whichever is present
            // (the previous hard-coded ".aax" meant AAXC retries always failed).
            val cacheDir = context.cacheDir
            val audiobooksDir = File(cacheDir, "audiobooks")
            val decryptedCachePath = File(audiobooksDir, "$asin.m4b").absolutePath
            val encryptedFile = listOf("aaxc", "aax", "aa")
                .map { File(audiobooksDir, "$asin.$it") }
                .firstOrNull { it.exists() }

            if (encryptedFile == null) {
                // No cached source left (validation failure deletes it): a retry can never
                // succeed. Mark cancelled instead of failed so the library shows the
                // download button again — failed + stored keys would keep showing a retry
                // button that loops forever.
                Log.e(TAG, "Encrypted file not found for retry: $asin (aaxc/aax) - resetting for re-download")
                updateTaskStatusInDb(taskId, "cancelled")
                return@withContext false
            }
            val encryptedPath = encryptedFile.absolutePath

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
