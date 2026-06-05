package expo.modules.rustbridge

import android.app.*
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import org.json.JSONObject
import kotlinx.coroutines.*

/**
 * Foreground Service for background downloads and conversions
 *
 * This service:
 * - Keeps downloads/conversions alive when app is backgrounded
 * - Shows persistent notification with progress
 * - Orchestrates download → conversion pipeline
 * - Handles lifecycle events and cleanup
 */
class DownloadService : Service() {
    companion object {
        private const val TAG = "DownloadService"
        private const val NOTIFICATION_CHANNEL_ID = "audiobook_downloads"
        private const val NOTIFICATION_ID = 1001

        private const val ACTION_ENQUEUE_DOWNLOAD = "expo.modules.rustbridge.ENQUEUE_DOWNLOAD"
        private const val ACTION_PAUSE_TASK = "expo.modules.rustbridge.PAUSE_TASK"
        private const val ACTION_RESUME_TASK = "expo.modules.rustbridge.RESUME_TASK"
        private const val ACTION_CANCEL_TASK = "expo.modules.rustbridge.CANCEL_TASK"
        private const val ACTION_STOP_MONITORING = "expo.modules.rustbridge.STOP_MONITORING"
        private const val ACTION_SET_WIFI_ONLY = "expo.modules.rustbridge.SET_WIFI_ONLY"
        private const val ACTION_RETRY_CONVERSION = "expo.modules.rustbridge.RETRY_CONVERSION"
        private const val ACTION_ENQUEUE_LIBRIVOX = "expo.modules.rustbridge.ENQUEUE_LIBRIVOX"

        private const val EXTRA_DB_PATH = "db_path"
        private const val EXTRA_ACCOUNT_JSON = "account_json"
        private const val EXTRA_ASIN = "asin"
        private const val EXTRA_TITLE = "title"
        private const val EXTRA_AUTHOR = "author"
        private const val EXTRA_DOWNLOAD_URL = "download_url"
        private const val EXTRA_OUTPUT_DIR = "output_dir"
        private const val EXTRA_QUALITY = "quality"
        private const val EXTRA_TASK_ID = "task_id"
        private const val EXTRA_WIFI_ONLY = "wifi_only"

        private fun startUserInitiatedService(context: Context, intent: Intent) {
            try {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                    context.startForegroundService(intent)
                } else {
                    context.startService(intent)
                }
            } catch (e: Exception) {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S &&
                    e.javaClass.name == "android.app.ForegroundServiceStartNotAllowedException"
                ) {
                    Log.e(TAG, "Blocked dataSync foreground service start; downloads must be started from a visible user action", e)
                }
                throw e
            }
        }

        /**
         * Enqueue a book download from a direct user action.
         */
        fun enqueueBook(
            context: Context,
            dbPath: String,
            accountJson: String,
            asin: String,
            title: String,
            outputDirectory: String,
            quality: String = "High"
        ) {
            val intent = Intent(context, DownloadService::class.java).apply {
                action = ACTION_ENQUEUE_DOWNLOAD
                putExtra(EXTRA_DB_PATH, dbPath)
                putExtra(EXTRA_ACCOUNT_JSON, accountJson)
                putExtra(EXTRA_ASIN, asin)
                putExtra(EXTRA_TITLE, title)
                putExtra(EXTRA_OUTPUT_DIR, outputDirectory)
                putExtra(EXTRA_QUALITY, quality)
            }

            startUserInitiatedService(context, intent)
        }

        /**
         * Pause a task
         */
        fun pauseTask(context: Context, taskId: String) {
            val intent = Intent(context, DownloadService::class.java).apply {
                action = ACTION_PAUSE_TASK
                putExtra(EXTRA_TASK_ID, taskId)
            }
            context.startService(intent)
        }

        /**
         * Resume a task
         */
        fun resumeTask(context: Context, dbPath: String, taskId: String) {
            val intent = Intent(context, DownloadService::class.java).apply {
                action = ACTION_RESUME_TASK
                putExtra(EXTRA_DB_PATH, dbPath)
                putExtra(EXTRA_TASK_ID, taskId)
            }
            context.startService(intent)
        }

        /**
         * Cancel a task
         */
        fun cancelTask(context: Context, dbPath: String, taskId: String) {
            val intent = Intent(context, DownloadService::class.java).apply {
                action = ACTION_CANCEL_TASK
                putExtra(EXTRA_DB_PATH, dbPath)
                putExtra(EXTRA_TASK_ID, taskId)
            }
            context.startService(intent)
        }

        /**
         * Retry conversion for a failed download
         */
        fun retryConversion(context: Context, dbPath: String, asin: String) {
            val intent = Intent(context, DownloadService::class.java).apply {
                action = ACTION_RETRY_CONVERSION
                putExtra(EXTRA_DB_PATH, dbPath)
                putExtra(EXTRA_ASIN, asin)
            }

            startUserInitiatedService(context, intent)
        }

        /**
         * Enqueue a LibriVox book download (no DRM, no decryption).
         */
        fun enqueueLibrivoxBook(
            context: Context,
            librivoxId: String,
            title: String,
            author: String,
            downloadUrl: String,
            outputDirectory: String
        ) {
            val intent = Intent(context, DownloadService::class.java).apply {
                action = ACTION_ENQUEUE_LIBRIVOX
                putExtra(EXTRA_ASIN, "librivox_$librivoxId")
                putExtra(EXTRA_TITLE, title)
                putExtra(EXTRA_AUTHOR, author)
                putExtra(EXTRA_DOWNLOAD_URL, downloadUrl)
                putExtra(EXTRA_OUTPUT_DIR, outputDirectory)
            }

            startUserInitiatedService(context, intent)
        }
    }

    private lateinit var orchestrator: DownloadOrchestrator
    private lateinit var notificationManager: DownloadNotificationManager
    private lateinit var dbPath: String
    private val serviceScope = CoroutineScope(Dispatchers.Main + SupervisorJob())
    private var isForeground = false

    // Track current download info for notifications
    private var currentDownload: DownloadInfo? = null

    data class DownloadInfo(
        val asin: String,
        val title: String,
        val author: String? = null,
        val totalBytes: Long = 0
    )

    override fun onCreate() {
        super.onCreate()
        Log.d(TAG, "Service created")

        // Get database path from intent or use default
        dbPath = AppPaths.databasePath(applicationContext)

        orchestrator = DownloadOrchestrator(applicationContext, dbPath)
        notificationManager = DownloadNotificationManager(applicationContext)

        // Set up orchestrator callbacks
        orchestrator.setProgressCallback { asin, stage, percentage, bytesDownloaded, totalBytes ->
            currentDownload?.let { download ->
                val progress = DownloadNotificationManager.DownloadProgress(
                    asin = download.asin,
                    title = download.title,
                    author = download.author,
                    stage = stage,
                    percentage = percentage.toInt(),
                    bytesDownloaded = bytesDownloaded,
                    totalBytes = totalBytes
                )
                notificationManager.showProgress(progress)
            }
        }

        orchestrator.setCompletionCallback { asin, title, outputPath ->
            currentDownload?.let { download ->
                notificationManager.showCompletion(download.title, download.author, outputPath)
            }
            currentDownload = null
            checkAndStopServiceIfIdle()
        }

        orchestrator.setErrorCallback { asin, title, error ->
            currentDownload?.let { download ->
                notificationManager.showError(download.title, download.author, error)
            }
            currentDownload = null
            checkAndStopServiceIfIdle()
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        Log.d(TAG, "onStartCommand: ${intent?.action}")

        if (intent == null) {
            Log.w(TAG, "Restarted without an intent; stopping to avoid sticky dataSync foreground work")
            stopSelf(startId)
            return START_NOT_STICKY
        }

        if (requiresForeground(intent.action)) {
            val initialNotification = notificationManager.getInitialNotification()
            Log.d(TAG, "Starting typed dataSync foreground service with notification")
            startDataSyncForeground(initialNotification)
        }

        when (intent.action) {
            ACTION_ENQUEUE_DOWNLOAD -> handleEnqueueDownload(intent)
            ACTION_PAUSE_TASK -> handlePauseTask(intent)
            ACTION_RESUME_TASK -> handleResumeTask(intent)
            ACTION_CANCEL_TASK -> handleCancelTask(intent)
            ACTION_STOP_MONITORING -> handleStopMonitoring(intent)
            ACTION_SET_WIFI_ONLY -> handleSetWifiOnly(intent)
            ACTION_RETRY_CONVERSION -> handleRetryConversion(intent)
            ACTION_ENQUEUE_LIBRIVOX -> handleEnqueueLibrivox(intent)
        }

        return START_NOT_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        super.onDestroy()
        Log.d(TAG, "Service destroyed")
        orchestrator.shutdown()
        serviceScope.cancel()
    }

    override fun onTimeout(startId: Int, fgsType: Int) {
        Log.w(TAG, "dataSync foreground service timed out (type=$fgsType); pausing downloads and stopping")
        if (::orchestrator.isInitialized) {
            runBlocking {
                withTimeoutOrNull(2_000) {
                    orchestrator.pauseActiveDownloadsForServiceTimeout()
                }
            }
        }
        stopForegroundCompat()
        stopSelf(startId)
    }

    private fun requiresForeground(action: String?): Boolean {
        return action == ACTION_ENQUEUE_DOWNLOAD || action == ACTION_RETRY_CONVERSION || action == ACTION_ENQUEUE_LIBRIVOX
    }

    private fun startDataSyncForeground(notification: Notification) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
        isForeground = true
    }

    private fun stopForegroundCompat() {
        if (!isForeground) return

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            stopForeground(STOP_FOREGROUND_REMOVE)
        } else {
            @Suppress("DEPRECATION")
            stopForeground(true)
        }
        isForeground = false
    }

    // ========================================================================
    // Intent Handlers
    // ========================================================================

    private fun handleEnqueueDownload(intent: Intent) {
        val accountJson = intent.getStringExtra(EXTRA_ACCOUNT_JSON) ?: return
        val asin = intent.getStringExtra(EXTRA_ASIN) ?: return
        val title = intent.getStringExtra(EXTRA_TITLE) ?: return
        val outputDir = intent.getStringExtra(EXTRA_OUTPUT_DIR) ?: return
        val quality = intent.getStringExtra(EXTRA_QUALITY) ?: "High"

        Log.d(TAG, "Enqueueing download via orchestrator: $asin - $title")

        // Store current download info for notifications
        currentDownload = DownloadInfo(
            asin = asin,
            title = title,
            author = null, // TODO: Pass author from intent
            totalBytes = 0
        )

        // Use service scope to call suspend function
        serviceScope.launch {
            try {
                orchestrator.enqueueBook(accountJson, asin, title, outputDir, quality)
                Log.d(TAG, "Book enqueued successfully: $asin")
            } catch (e: Exception) {
                Log.e(TAG, "Failed to enqueue book", e)
                currentDownload?.let { download ->
                    notificationManager.showError(download.title, download.author, e.message ?: "Unknown error")
                }
                currentDownload = null
            }
        }
    }

    private fun handlePauseTask(intent: Intent) {
        val taskId = intent.getStringExtra(EXTRA_TASK_ID) ?: return
        Log.d(TAG, "Pausing download: $taskId")

        try {
            val pauseParams = JSONObject().apply {
                put("db_path", dbPath)
                put("task_id", taskId)
            }
            ExpoRustBridgeModule.nativePauseDownload(pauseParams.toString())
        } catch (e: Exception) {
            Log.e(TAG, "Failed to pause download", e)
        }
    }

    private fun handleResumeTask(intent: Intent) {
        val taskId = intent.getStringExtra(EXTRA_TASK_ID) ?: return
        Log.d(TAG, "Resuming download: $taskId")

        try {
            val resumeParams = JSONObject().apply {
                put("db_path", dbPath)
                put("task_id", taskId)
            }
            ExpoRustBridgeModule.nativeResumeDownload(resumeParams.toString())
        } catch (e: Exception) {
            Log.e(TAG, "Failed to resume download", e)
        }
    }

    private fun handleCancelTask(intent: Intent) {
        val taskId = intent.getStringExtra(EXTRA_TASK_ID) ?: return
        Log.d(TAG, "Cancelling download: $taskId")

        try {
            val cancelParams = JSONObject().apply {
                put("db_path", dbPath)
                put("task_id", taskId)
            }
            ExpoRustBridgeModule.nativeCancelDownload(cancelParams.toString())
        } catch (e: Exception) {
            Log.e(TAG, "Failed to cancel download", e)
        }
    }

    private fun handleRetryConversion(intent: Intent) {
        val asin = intent.getStringExtra(EXTRA_ASIN) ?: return
        Log.d(TAG, "Retrying conversion for: $asin")

        serviceScope.launch {
            try {
                val success = orchestrator.retryConversion(asin)
                if (!success) {
                    Log.e(TAG, "Retry conversion failed for $asin")
                }
            } catch (e: Exception) {
                Log.e(TAG, "Error retrying conversion", e)
            }
        }
    }

    private fun handleEnqueueLibrivox(intent: Intent) {
        val asin = intent.getStringExtra(EXTRA_ASIN) ?: return
        val title = intent.getStringExtra(EXTRA_TITLE) ?: return
        val author = intent.getStringExtra(EXTRA_AUTHOR) ?: ""
        val downloadUrl = intent.getStringExtra(EXTRA_DOWNLOAD_URL) ?: return
        val outputDir = intent.getStringExtra(EXTRA_OUTPUT_DIR) ?: return

        // Extract librivoxId from the "librivox_" prefixed asin
        val librivoxId = asin.removePrefix("librivox_")

        Log.d(TAG, "Enqueueing LibriVox download: $asin - $title")

        currentDownload = DownloadInfo(
            asin = asin,
            title = title,
            author = author,
            totalBytes = 0
        )

        serviceScope.launch {
            try {
                orchestrator.enqueueLibrivoxBook(librivoxId, title, author, downloadUrl, outputDir)
                Log.d(TAG, "LibriVox book enqueued successfully: $asin")
            } catch (e: Exception) {
                Log.e(TAG, "Failed to enqueue LibriVox book", e)
                currentDownload?.let { download ->
                    notificationManager.showError(download.title, download.author, e.message ?: "Unknown error")
                }
                currentDownload = null
            }
        }
    }

    private fun handleStopMonitoring(intent: Intent) {
        val asin = intent.getStringExtra("asin") ?: return
        Log.d(TAG, "Stopping monitoring for: $asin")
        orchestrator.stopMonitoring(asin)

        // Check if there are any active downloads left
        checkAndStopServiceIfIdle()
    }

    /**
     * Check if service should stop (no active downloads)
     */
    private fun checkAndStopServiceIfIdle() {
        try {
            // List all tasks (no status filter) so we count queued and in-progress
            // work, not just actively downloading tasks. Stopping the service while
            // tasks are still queued cancels the service scope and breaks the queue.
            val listParams = JSONObject().apply {
                put("db_path", dbPath)
            }

            val listResult = ExpoRustBridgeModule.nativeListDownloadTasks(listParams.toString())
            val json = JSONObject(listResult)

            if (json.getBoolean("success")) {
                val data = json.getJSONObject("data")
                val tasks = data.getJSONArray("tasks")

                val activeStatuses = setOf(
                    "queued", "downloading", "decrypting", "validating", "copying"
                )
                var activeCount = 0
                for (i in 0 until tasks.length()) {
                    if (tasks.getJSONObject(i).optString("status") in activeStatuses) {
                        activeCount++
                    }
                }

                if (activeCount == 0) {
                    Log.d(TAG, "No active downloads remaining - stopping service")
                    stopForegroundCompat()
                    stopSelf()
                } else {
                    Log.d(TAG, "$activeCount downloads still active - keeping service alive")
                }
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error checking active downloads", e)
        }
    }

    private fun handleSetWifiOnly(intent: Intent) {
        val wifiOnly = intent.getBooleanExtra(EXTRA_WIFI_ONLY, false)
        Log.d(TAG, "Setting WiFi-only mode: $wifiOnly")
        orchestrator.setWifiOnlyMode(wifiOnly)
    }

    /**
     * Public helper to stop monitoring from broadcast receiver
     */
    fun stopMonitoringForAsin(asin: String) {
        orchestrator.stopMonitoring(asin)
    }
}
