package expo.modules.rustbridge

import android.app.*
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import androidx.core.app.NotificationCompat
import android.util.Log
import java.util.concurrent.ConcurrentHashMap

/**
 * Rich notification manager for audiobook downloads
 *
 * Features:
 * - Progress bar with percentage
 * - Book title and author
 * - Current stage (Downloading, Decrypting, Copying)
 * - Action buttons (Pause/Cancel)
 * - Large text style for detailed info
 * - Different notifications for different stages
 */
class DownloadNotificationManager(private val context: Context) {
    companion object {
        private const val TAG = "DownloadNotification"
        private const val CHANNEL_ID = "audiobook_downloads"
        private const val CHANNEL_NAME = "Audiobook Downloads"
        private const val NOTIFICATION_ID = 1001

        /**
         * Stable per-book notification id. Deterministic (hash-based) so a manager
         * created in the service and one created in the broadcast receiver agree on
         * which notification belongs to a given ASIN. High base avoids the foreground
         * anchor (1001) and its neighbours.
         */
        fun notificationIdFor(asin: String): Int = 100_000 + asin.hashCode().mod(800_000)

        /**
         * When each book's progress notification first appeared. Android orders
         * same-priority notifications by their timestamp, which defaults to "now" on every
         * update, so concurrent downloads kept swapping places as they reported progress.
         * Pinning it per book keeps the order stable. Shared across manager instances.
         */
        private val shownSince = ConcurrentHashMap<String, Long>()

        /** Opens Settings → Downloads; see `linking` in AppNavigator.tsx. */
        private const val DOWNLOADS_LINK = "librisync://downloads"
        @Volatile private var summaryShownSince = 0L

        // Action request codes
        private const val ACTION_PAUSE = "expo.modules.rustbridge.PAUSE_DOWNLOAD"
        private const val ACTION_RESUME = "expo.modules.rustbridge.RESUME_DOWNLOAD"
        private const val ACTION_CANCEL = "expo.modules.rustbridge.CANCEL_DOWNLOAD"
        private const val ACTION_RETRY = "expo.modules.rustbridge.RETRY_DOWNLOAD"

        // Notification types
        const val STAGE_DOWNLOADING = "downloading"
        const val STAGE_DECRYPTING = "decrypting"
        const val STAGE_COPYING = "copying"
        const val STAGE_ENCODING = "encoding"
        const val STAGE_JOINING = "joining"
        const val STAGE_COMPLETED = "completed"
        const val STAGE_FAILED = "failed"
    }

    private val notificationManager = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager

    init {
        createNotificationChannel()
    }

    /**
     * Download progress state
     */
    data class DownloadProgress(
        val asin: String,
        val title: String,
        val author: String? = null,
        val stage: String,
        val percentage: Int,
        val bytesDownloaded: Long = 0,
        val totalBytes: Long = 0,
        val speed: String? = null, // e.g., "2.5 MB/s"
        val eta: String? = null // e.g., "5 minutes remaining"
    )

    /**
     * Create notification channel (Android O+)
     */
    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                CHANNEL_NAME,
                NotificationManager.IMPORTANCE_LOW
            ).apply {
                description = "Shows progress for audiobook downloads, decryption, and conversion"
                setShowBadge(false)
                enableLights(false)
                enableVibration(false)
            }
            notificationManager.createNotificationChannel(channel)
        }
    }

    /**
     * Show download progress notification
     */
    fun showProgress(progress: DownloadProgress) {
        Log.d(TAG, "Showing progress notification: ${progress.title} ${progress.percentage}%")
        val notification = buildProgressNotification(progress)
        notificationManager.notify(notificationIdFor(progress.asin), notification)
    }

    /**
     * Build progress notification
     */
    private fun buildProgressNotification(progress: DownloadProgress): Notification {
        val stageName = when (progress.stage) {
            STAGE_DOWNLOADING -> "Downloading"
            STAGE_DECRYPTING -> "Decrypting"
            STAGE_COPYING -> "Saving to library"
            STAGE_ENCODING -> "Converting to MP3"
            STAGE_JOINING -> "Joining tracks"
            else -> "Processing"
        }

        val title = "$stageName: ${progress.title}"

        // Build detailed content text
        val contentText = buildString {
            append("${progress.percentage}%")

            if (progress.totalBytes > 0) {
                val mbDownloaded = progress.bytesDownloaded / (1024.0 * 1024.0)
                val mbTotal = progress.totalBytes / (1024.0 * 1024.0)
                append(" • %.1f / %.1f MB".format(mbDownloaded, mbTotal))
            }

            progress.speed?.let { append(" • $it") }
            progress.eta?.let { append(" • $it") }
        }

        // Build big text with more details
        val bigText = buildString {
            append("$stageName ${progress.title}")
            progress.author?.let { append("\nby $it") }
            append("\n\n${progress.percentage}% complete")

            if (progress.totalBytes > 0) {
                val mbDownloaded = progress.bytesDownloaded / (1024.0 * 1024.0)
                val mbTotal = progress.totalBytes / (1024.0 * 1024.0)
                append("\n%.1f MB of %.1f MB".format(mbDownloaded, mbTotal))
            }

            progress.speed?.let { append("\nSpeed: $it") }
            progress.eta?.let { append("\n$it") }
        }

        val builder = NotificationCompat.Builder(context, CHANNEL_ID)
            .setContentTitle(title)
            .setContentText(contentText)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setProgress(100, progress.percentage, false)
            .setOngoing(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setStyle(NotificationCompat.BigTextStyle().bigText(bigText))
            .setOnlyAlertOnce(true) // Don't make sound/vibration on updates
            .setWhen(shownSince.getOrPut(progress.asin) { System.currentTimeMillis() })
            .setShowWhen(false)
            .setContentIntent(openAppIntent())

        // Per-ASIN request codes: a constant code + FLAG_UPDATE_CURRENT makes every
        // notification's button share one PendingIntent, so its extras (asin) get
        // overwritten by the last one built — cancelling then hits the wrong book.
        val reqBase = notificationIdFor(progress.asin) * 2

        // Pause only makes sense while downloading (the Rust download supports resume).
        if (progress.stage == STAGE_DOWNLOADING) {
            val pauseIntent = Intent(ACTION_PAUSE).apply {
                setPackage(context.packageName)
                putExtra("asin", progress.asin)
            }
            val pausePendingIntent = PendingIntent.getBroadcast(
                context,
                reqBase,
                pauseIntent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
            )
            builder.addAction(
                android.R.drawable.ic_media_pause,
                "Pause",
                pausePendingIntent
            )
        }

        // Cancel is available in every stage, including decrypt / validate / copy.
        val cancelIntent = Intent(ACTION_CANCEL).apply {
            setPackage(context.packageName)
            putExtra("asin", progress.asin)
        }
        val cancelPendingIntent = PendingIntent.getBroadcast(
            context,
            reqBase + 1,
            cancelIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
        builder.addAction(
            android.R.drawable.ic_menu_close_clear_cancel,
            "Cancel",
            cancelPendingIntent
        )

        return builder.build()
    }

    /**
     * Show completion notification
     */
    fun showCompletion(asin: String, title: String, author: String? = null, outputPath: String) {
        val contentText = buildString {
            append("Ready to listen")
            author?.let { append(" • by $it") }
        }

        val bigText = buildString {
            append("$title is ready to listen!")
            author?.let { append("\n\nby $it") }
            append("\n\nSaved to your library")
        }

        val notification = NotificationCompat.Builder(context, CHANNEL_ID)
            .setContentTitle("Download Complete: $title")
            .setContentText(contentText)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setAutoCancel(true)
            .setPriority(NotificationCompat.PRIORITY_DEFAULT)
            .setStyle(NotificationCompat.BigTextStyle().bigText(bigText))
            .setContentIntent(openAppIntent())
            .build()
        shownSince.remove(asin)

        // Replace this book's ongoing progress notification in place.
        notificationManager.notify(notificationIdFor(asin), notification)

        Log.d(TAG, "Completion notification shown: $title")
    }

    /**
     * Show error notification
     */
    fun showError(asin: String, title: String, author: String? = null, error: String) {
        val contentText = buildString {
            append("Failed: $error")
        }

        val bigText = buildString {
            append("$title")
            author?.let { append("\nby $it") }
            append("\n\nFailed: $error")
        }

        // Retry action: re-runs the conversion from the cached encrypted file.
        val retryIntent = Intent(ACTION_RETRY).apply {
            setPackage(context.packageName)
            putExtra("asin", asin)
        }
        val retryPendingIntent = PendingIntent.getBroadcast(
            context,
            notificationIdFor(asin) * 2 + 1,
            retryIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        val notification = NotificationCompat.Builder(context, CHANNEL_ID)
            .setContentTitle("Download Failed: $title")
            .setContentText(contentText)
            .setSmallIcon(android.R.drawable.stat_notify_error)
            .setAutoCancel(true)
            .setPriority(NotificationCompat.PRIORITY_DEFAULT)
            .setStyle(NotificationCompat.BigTextStyle().bigText(bigText))
            .addAction(android.R.drawable.ic_menu_rotate, "Retry", retryPendingIntent)
            .setContentIntent(openAppIntent())
            .build()
        shownSince.remove(asin)

        // Replace this book's ongoing progress notification in place.
        notificationManager.notify(notificationIdFor(asin), notification)

        Log.e(TAG, "Error notification shown: $title - $error")
    }

    /**
     * Show paused notification
     */
    fun showPaused(asin: String, title: String, author: String? = null, percentage: Int) {
        Log.d(TAG, "Showing paused notification: $title at $percentage%")

        val contentText = buildString {
            append("Paused at $percentage%")
            author?.let { append(" • by $it") }
        }

        val bigText = buildString {
            append("$title")
            author?.let { append("\nby $it") }
            append("\n\nPaused at $percentage%")
            append("\nTap Resume to continue or Cancel to remove")
        }

        val builder = NotificationCompat.Builder(context, CHANNEL_ID)
            .setContentTitle("Download Paused")
            .setContentText(contentText)
            .setSmallIcon(android.R.drawable.ic_media_pause)
            .setProgress(100, percentage, false)
            .setAutoCancel(false)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setStyle(NotificationCompat.BigTextStyle().bigText(bigText))
            .setContentIntent(openAppIntent())

        // Resume button
        val resumeIntent = Intent(ACTION_RESUME).apply {
            setPackage(context.packageName)
            putExtra("asin", asin)
        }
        val reqBase = notificationIdFor(asin) * 2
        val resumePendingIntent = PendingIntent.getBroadcast(
            context,
            reqBase,
            resumeIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
        builder.addAction(
            android.R.drawable.ic_media_play,
            "Resume",
            resumePendingIntent
        )

        // Cancel button
        val cancelIntent = Intent(ACTION_CANCEL).apply {
            setPackage(context.packageName)
            putExtra("asin", asin)
        }
        val cancelPendingIntent = PendingIntent.getBroadcast(
            context,
            reqBase + 1,
            cancelIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
        builder.addAction(
            android.R.drawable.ic_menu_close_clear_cancel,
            "Cancel",
            cancelPendingIntent
        )

        notificationManager.notify(NOTIFICATION_ID, builder.build())
        Log.d(TAG, "Paused notification shown with Resume and Cancel buttons")
    }

    /**
     * Cancel all notifications
     */
    /** Cancel just one book's notification (per-book cancel from its own action button). */
    fun cancelForAsin(asin: String) {
        notificationManager.cancel(notificationIdFor(asin))
        shownSince.remove(asin)
        Log.d(TAG, "Cancelled notification for $asin")
    }

    fun cancelAll() {
        Log.d(TAG, "Cancelling all notifications")
        notificationManager.cancel(NOTIFICATION_ID)
        notificationManager.cancel(NOTIFICATION_ID + 1)
        notificationManager.cancel(NOTIFICATION_ID + 2)
        Log.d(TAG, "All notifications cancelled")
    }

    /**
     * Update the ongoing foreground-anchor notification (id NOTIFICATION_ID) with the
     * count of active downloads. Per-book detail lives in the per-ASIN notifications.
     */
    fun showSummary(activeCount: Int, queuedTitles: List<String> = emptyList()) {
        val text = buildString {
            append(if (activeCount == 1) "Downloading 1 audiobook" else "Downloading $activeCount audiobooks")
            if (queuedTitles.isNotEmpty()) append(" • ${queuedTitles.size} queued")
        }
        val bigText = buildString {
            append(text)
            if (queuedTitles.isNotEmpty()) {
                append("\n\nQueued:")
                queuedTitles.forEach { append("\n• $it") }
            }
        }
        val notification = NotificationCompat.Builder(context, CHANNEL_ID)
            .setContentTitle("LibriSync")
            .setContentText(text)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setOngoing(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setOnlyAlertOnce(true)
            .setStyle(NotificationCompat.BigTextStyle().bigText(bigText))
            .setWhen(summaryWhen())
            .setShowWhen(false)
            .setContentIntent(openAppIntent())
            .build()
        notificationManager.notify(NOTIFICATION_ID, notification)
    }

    /**
     * Cancel just the ongoing summary anchor (id NOTIFICATION_ID). Needed because the
     * anchor is posted via notify(), so stopForeground(REMOVE) won't clear it when the
     * service isn't in the foreground state (e.g. the WorkManager-driven path) — leaving
     * a stale "Downloading N audiobooks" after everything has finished.
     */
    fun cancelSummary() {
        notificationManager.cancel(NOTIFICATION_ID)
        summaryShownSince = 0L
        Log.d(TAG, "Cancelled download summary notification")
    }

    /**
     * Get initial notification for starting foreground service
     */
    fun getInitialNotification(): Notification {
        return NotificationCompat.Builder(context, CHANNEL_ID)
            .setContentTitle("Audiobook Download")
            .setContentText("Initializing...")
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setProgress(0, 0, true)
            .setOngoing(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setContentIntent(openAppIntent())
            .build()
    }

    /** The summary keeps one timestamp while downloads run, for the same reason as books. */
    private fun summaryWhen(): Long {
        if (summaryShownSince == 0L) summaryShownSince = System.currentTimeMillis()
        return summaryShownSince
    }

    /**
     * Tapping any download notification opens the Downloads screen (Settings → Downloads),
     * where every book's progress or error is listed. It's a deep link handled by the app's
     * navigator (scheme declared in app.config.js), pinned to this package so the debug
     * and release apps don't both claim it. Falls back to plain launch if nothing handles it.
     */
    private fun openAppIntent(): PendingIntent? {
        val downloads = Intent(Intent.ACTION_VIEW, Uri.parse(DOWNLOADS_LINK))
            .setPackage(context.packageName)
        val launch = if (downloads.resolveActivity(context.packageManager) != null) {
            downloads
        } else {
            context.packageManager.getLaunchIntentForPackage(context.packageName) ?: return null
        }
        launch.flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP
        return PendingIntent.getActivity(
            context,
            0,
            launch,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
    }
}
