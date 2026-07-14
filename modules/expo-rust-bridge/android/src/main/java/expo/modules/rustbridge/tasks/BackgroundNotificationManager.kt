package expo.modules.rustbridge.tasks

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build
import android.util.Log
import androidx.core.app.NotificationCompat

/**
 * Unified notification manager for all background tasks
 *
 * Shows a single notification that displays all active tasks:
 * - Downloads with progress
 * - Library sync status
 * - Token refresh status
 * - Auto-download progress
 */
class BackgroundNotificationManager(private val context: Context) {
    companion object {
        private const val TAG = "BackgroundNotification"
        const val CHANNEL_ID = "background_tasks"
        const val CHANNEL_NAME = "Background Tasks"
        const val NOTIFICATION_ID = 2000

        // Actions
        private const val ACTION_PAUSE_ALL = "expo.modules.rustbridge.PAUSE_ALL"
        private const val ACTION_CANCEL_ALL = "expo.modules.rustbridge.CANCEL_ALL"
    }

    private val notificationManager = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager

    init {
        createNotificationChannel()
    }

    /**
     * Create notification channel (Android O+)
     */
    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                CHANNEL_NAME,
                NotificationManager.IMPORTANCE_MIN  // MIN = completely hidden when idle
            ).apply {
                description = "Shows status of background tasks including downloads, library sync, and token refresh"
                setShowBadge(false)
                enableLights(false)
                enableVibration(false)
                setSound(null, null)  // No sound
                lockscreenVisibility = Notification.VISIBILITY_SECRET  // Hidden on lockscreen
            }
            notificationManager.createNotificationChannel(channel)
        }
    }

    /**
     * Build notification showing all active tasks
     */
    fun buildNotification(tasks: List<Task>): Notification {
        Log.d(TAG, "Building notification for ${tasks.size} active tasks")

        val activeTasks = tasks.filter {
            it.status == TaskStatus.RUNNING || it.status == TaskStatus.PENDING || it.status == TaskStatus.PAUSED
        }

        // Build title
        val title = when {
            activeTasks.isEmpty() -> "LibriSync Background Service"
            activeTasks.size == 1 -> {
                val task = activeTasks.first()
                when (task.type) {
                    TaskType.DOWNLOAD -> "Downloading audiobook"
                    TaskType.LIBRARY_SYNC -> "Syncing library"
                    TaskType.TOKEN_REFRESH -> "Refreshing token"
                    TaskType.AUTO_DOWNLOAD -> "Auto-downloading books"
                }
            }
            else -> "LibriSync: ${activeTasks.size} active tasks"
        }

        // Build content text (summary)
        val contentText = when {
            activeTasks.isEmpty() -> "Ready for background tasks"
            activeTasks.size == 1 -> getTaskSummary(activeTasks.first())
            else -> "${activeTasks.count { it.type == TaskType.DOWNLOAD }} downloads, " +
                    "${activeTasks.count { it.type == TaskType.LIBRARY_SYNC }} syncs"
        }

        // Build big text (detailed list)
        val bigText = buildString {
            if (activeTasks.isEmpty()) {
                append("No active tasks\n\n")
                append("Background service is running and ready to handle:\n")
                append("• Audiobook downloads\n")
                append("• Library synchronization\n")
                append("• Automatic token refresh\n")
                append("• Automatic downloads")
            } else {
                append("Active tasks:\n\n")
                activeTasks.forEach { task ->
                    append("• ${getTaskDescription(task)}\n")
                }
            }
        }

        // Choose icon based on activity
        val hasActiveDownload = activeTasks.any { it.type == TaskType.DOWNLOAD && it.status == TaskStatus.RUNNING }
        val icon = if (hasActiveDownload) {
            android.R.drawable.stat_sys_download // Animated download icon when downloading
        } else {
            android.R.drawable.stat_notify_sync // Sync icon when idle or paused
        }

        // Use MIN priority when idle to completely hide notification
        // Use LOW priority when there are tasks to show in status bar
        val priority = if (activeTasks.isEmpty()) {
            NotificationCompat.PRIORITY_MIN  // Completely hidden
        } else {
            NotificationCompat.PRIORITY_LOW   // Visible in status bar
        }

        // When idle, allow dismissal. When active, prevent accidental dismissal.
        val ongoing = activeTasks.isNotEmpty()

        val builder = NotificationCompat.Builder(context, CHANNEL_ID)
            .setContentTitle(title)
            .setContentText(contentText)
            .setSmallIcon(icon)
            .setOngoing(ongoing)  // Swipable when idle, locked when active
            .setPriority(priority)
            .setStyle(NotificationCompat.BigTextStyle().bigText(bigText))
            .setOnlyAlertOnce(true)
            .setSilent(true)  // Completely silent, no sound or vibration

        // When idle, make notification invisible everywhere (status bar + notification drawer)
        if (activeTasks.isEmpty()) {
            builder.setVisibility(NotificationCompat.VISIBILITY_SECRET)
        }

        // Add progress bar if there's a download
        val downloadTask = activeTasks.firstOrNull { it.type == TaskType.DOWNLOAD }
        if (downloadTask != null) {
            val percentage = downloadTask.getMetadataInt(DownloadTaskMetadata.PERCENTAGE) ?: 0
            builder.setProgress(100, percentage, false)
        }

        return builder.build()
    }

    private fun formatEta(seconds: Int): String {
        val h = seconds / 3600
        val m = (seconds % 3600) / 60
        val s = seconds % 60
        return when {
            h > 0 -> "~${h}h ${m}m left"
            m > 0 -> "~${m}m ${s}s left"
            else -> "~${s}s left"
        }
    }

    /**
     * Get one-line summary for a task
     */
    private fun getTaskSummary(task: Task): String {
        return when (task.type) {
            TaskType.DOWNLOAD -> {
                val title = task.getMetadataString(DownloadTaskMetadata.TITLE) ?: "Audiobook"
                val percentage = task.getMetadataInt(DownloadTaskMetadata.PERCENTAGE) ?: 0
                val stage = task.getMetadataString(DownloadTaskMetadata.STAGE) ?: "downloading"
                when {
                    task.status == TaskStatus.PAUSED -> "Paused at $percentage% - $title"
                    stage == "downloading" -> {
                        val eta = task.getMetadataInt(DownloadTaskMetadata.ETA_SECONDS) ?: 0
                        if (eta > 0) "$percentage% - $title • ${formatEta(eta)}"
                        else "$percentage% - $title"
                    }
                    stage == "decrypting" -> {
                        val eta = task.getMetadataInt(DownloadTaskMetadata.ETA_SECONDS) ?: 0
                        if (eta > 0) "Decrypting $title ($percentage% • ${formatEta(eta)})"
                        else "Decrypting $title ($percentage%)"
                    }
                    stage == "validating" -> {
                        val eta = task.getMetadataInt(DownloadTaskMetadata.ETA_SECONDS) ?: 0
                        if (eta > 0) "Validating $title ($percentage% • ${formatEta(eta)})"
                        else "Validating $title ($percentage%)"
                    }
                    stage == "copying" -> {
                        val eta = task.getMetadataInt(DownloadTaskMetadata.ETA_SECONDS) ?: 0
                        if (eta > 0) "Saving $title ($percentage% • ${formatEta(eta)})"
                        else "Saving $title ($percentage%)"
                    }
                    else -> title
                }
            }
            TaskType.LIBRARY_SYNC -> {
                val page = task.getMetadataInt(LibrarySyncMetadata.CURRENT_PAGE) ?: 0
                val items = task.getMetadataInt(LibrarySyncMetadata.ITEMS_SYNCED) ?: 0
                "Syncing library (page $page, $items items)"
            }
            TaskType.TOKEN_REFRESH -> "Refreshing access token"
            TaskType.AUTO_DOWNLOAD -> "Checking for new books to download"
        }
    }

    /**
     * Get detailed description for a task
     */
    private fun getTaskDescription(task: Task): String {
        return when (task.type) {
            TaskType.DOWNLOAD -> {
                val title = task.getMetadataString(DownloadTaskMetadata.TITLE) ?: "Audiobook"
                val percentage = task.getMetadataInt(DownloadTaskMetadata.PERCENTAGE) ?: 0
                val stage = task.getMetadataString(DownloadTaskMetadata.STAGE) ?: "downloading"
                val bytesDownloaded = task.getMetadataLong(DownloadTaskMetadata.BYTES_DOWNLOADED) ?: 0L
                val totalBytes = task.getMetadataLong(DownloadTaskMetadata.TOTAL_BYTES) ?: 0L

                when {
                    task.status == TaskStatus.PAUSED -> {
                        val mbDownloaded = bytesDownloaded / (1024.0 * 1024.0)
                        val mbTotal = totalBytes / (1024.0 * 1024.0)
                        if (totalBytes > 0) {
                            String.format("PAUSED - %s: %d%% (%.1f / %.1f MB)", title, percentage, mbDownloaded, mbTotal)
                        } else {
                            "PAUSED - $title: $percentage%"
                        }
                    }
                    stage == "downloading" -> {
                        val mbDownloaded = bytesDownloaded / (1024.0 * 1024.0)
                        val mbTotal = totalBytes / (1024.0 * 1024.0)
                        val eta = task.getMetadataInt(DownloadTaskMetadata.ETA_SECONDS) ?: 0
                        val etaSuffix = if (eta > 0) " • ${formatEta(eta)}" else ""
                        if (totalBytes > 0) {
                            String.format("%s: %d%% (%.1f / %.1f MB)%s", title, percentage, mbDownloaded, mbTotal, etaSuffix)
                        } else {
                            "$title: $percentage%$etaSuffix"
                        }
                    }
                    stage == "decrypting" -> {
                        val eta = task.getMetadataInt(DownloadTaskMetadata.ETA_SECONDS) ?: 0
                        if (eta > 0) "Decrypting: $title • $percentage% • ${formatEta(eta)}"
                        else "Decrypting: $title • $percentage%"
                    }
                    stage == "validating" -> {
                        val eta = task.getMetadataInt(DownloadTaskMetadata.ETA_SECONDS) ?: 0
                        if (eta > 0) "Validating: $title • $percentage% • ${formatEta(eta)}"
                        else "Validating: $title • $percentage%"
                    }
                    stage == "copying" -> {
                        val eta = task.getMetadataInt(DownloadTaskMetadata.ETA_SECONDS) ?: 0
                        if (eta > 0) "Saving: $title • $percentage% • ${formatEta(eta)}"
                        else "Saving: $title • $percentage%"
                    }
                    else -> title
                }
            }
            TaskType.LIBRARY_SYNC -> {
                val page = task.getMetadataInt(LibrarySyncMetadata.CURRENT_PAGE) ?: 0
                val items = task.getMetadataInt(LibrarySyncMetadata.ITEMS_SYNCED) ?: 0
                val added = task.getMetadataInt(LibrarySyncMetadata.ITEMS_ADDED) ?: 0
                "Library sync: page $page ($items synced, $added new)"
            }
            TaskType.TOKEN_REFRESH -> "Refreshing access token"
            TaskType.AUTO_DOWNLOAD -> "Auto-downloading books"
        }
    }

    /**
     * Show notification
     */
    fun show(tasks: List<Task>) {
        val notification = buildNotification(tasks)
        notificationManager.notify(NOTIFICATION_ID, notification)
    }

    /**
     * Cancel notification
     */
    fun cancel() {
        Log.d(TAG, "Cancelling notification")
        notificationManager.cancel(NOTIFICATION_ID)
    }

    /**
     * Build initial notification for service startup
     */
    fun getInitialNotification(): Notification {
        return NotificationCompat.Builder(context, CHANNEL_ID)
            .setContentTitle("LibriSync Background Service")
            .setContentText("Initializing...")
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setOngoing(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .build()
    }
}
