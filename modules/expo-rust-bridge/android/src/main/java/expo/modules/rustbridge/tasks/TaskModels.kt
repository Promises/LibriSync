package expo.modules.rustbridge.tasks

import java.util.Date

/**
 * Task priority levels
 */
enum class TaskPriority(val value: Int) {
    CRITICAL(0),  // Token refresh about to expire
    HIGH(1),      // User-initiated download
    MEDIUM(2),    // Auto-download
    LOW(3);       // Background sync

    companion object {
        fun fromValue(value: Int) = values().find { it.value == value } ?: LOW
    }
}

/**
 * Task types
 */
enum class TaskType {
    DOWNLOAD,
    TOKEN_REFRESH,
    LIBRARY_SYNC,
    AUTO_DOWNLOAD
}

/**
 * Task status
 */
enum class TaskStatus {
    PENDING,      // Queued, waiting to start
    RUNNING,      // Currently executing
    PAUSED,       // Temporarily paused
    COMPLETED,    // Successfully finished
    FAILED,       // Failed with error
    CANCELLED     // Cancelled by user
}

/**
 * Base task representation
 */
data class Task(
    val id: String,
    val type: TaskType,
    val priority: TaskPriority,
    var status: TaskStatus,
    val metadata: MutableMap<String, Any> = mutableMapOf(),
    val createdAt: Date = Date(),
    var startedAt: Date? = null,
    var completedAt: Date? = null,
    var error: String? = null
) : Comparable<Task> {
    override fun compareTo(other: Task): Int {
        // Lower priority value = higher priority
        return this.priority.value.compareTo(other.priority.value)
    }

    fun getMetadataString(key: String): String? = metadata[key] as? String
    fun getMetadataInt(key: String): Int? = (metadata[key] as? Number)?.toInt()
    fun getMetadataLong(key: String): Long? = (metadata[key] as? Number)?.toLong()
    fun getMetadataBoolean(key: String): Boolean? = metadata[key] as? Boolean
}

/**
 * Download-specific task metadata keys
 */
object DownloadTaskMetadata {
    const val ASIN = "asin"
    const val TITLE = "title"
    const val AUTHOR = "author"
    const val DOWNLOAD_URL = "download_url"
    const val OUTPUT_DIR = "output_dir"
    const val TOTAL_BYTES = "total_bytes"
    const val BYTES_DOWNLOADED = "bytes_downloaded"
    const val PERCENTAGE = "percentage"
    const val ETA_SECONDS = "eta_seconds" // remaining seconds for current stage, 0 = unknown
    const val STAGE = "stage" // "downloading", "decrypting", "copying"
    const val ENCRYPTED_PATH = "encrypted_path"
    const val DECRYPTED_PATH = "decrypted_path"
    const val FILE_TYPE = "file_type"
    const val FILE_EXTENSION = "file_extension"
    const val AAXC_KEY = "aaxc_key"
    const val AAXC_IV = "aaxc_iv"
    const val RUST_TASK_ID = "rust_task_id"
}

/**
 * Token refresh task metadata keys
 */
object TokenRefreshMetadata {
    const val COUNTRY_CODE = "country_code"
    const val REFRESH_TOKEN = "refresh_token"
    const val DEVICE_SERIAL = "device_serial"
    const val EXPIRES_AT = "expires_at"
}

/**
 * Library sync task metadata keys
 */
object LibrarySyncMetadata {
    const val FULL_SYNC = "full_sync"
    const val CURRENT_PAGE = "current_page"
    const val TOTAL_PAGES = "total_pages"
    const val ITEMS_SYNCED = "items_synced"
    const val ITEMS_ADDED = "items_added"
    const val ITEMS_UPDATED = "items_updated"
}

/**
 * Event bus events for task updates
 */
sealed class TaskEvent {
    // Generic task events
    data class TaskStarted(val task: Task) : TaskEvent()
    data class TaskCompleted(val task: Task) : TaskEvent()
    data class TaskFailed(val task: Task, val error: String) : TaskEvent()
    data class TaskCancelled(val task: Task) : TaskEvent()
    data class TaskPaused(val task: Task) : TaskEvent()
    data class TaskResumed(val task: Task) : TaskEvent()

    // Download-specific events
    data class DownloadProgress(
        val taskId: String,
        val asin: String,
        val title: String,
        val stage: String,
        val percentage: Int,
        val bytesDownloaded: Long,
        val totalBytes: Long
    ) : TaskEvent()

    data class DownloadComplete(
        val taskId: String,
        val asin: String,
        val title: String,
        val outputPath: String
    ) : TaskEvent()

    // Token refresh events
    data class TokenRefreshStarted(val taskId: String) : TaskEvent()
    data class TokenRefreshed(val taskId: String, val newExpiry: Date) : TaskEvent()
    data class TokenRefreshFailed(val taskId: String, val error: String) : TaskEvent()

    // Library sync events
    data class LibrarySyncStarted(val taskId: String, val fullSync: Boolean) : TaskEvent()
    data class LibrarySyncProgress(
        val taskId: String,
        val page: Int,
        val totalItems: Int,
        val itemsAdded: Int,
        val itemsUpdated: Int
    ) : TaskEvent()
    data class LibrarySyncComplete(
        val taskId: String,
        val totalItems: Int,
        val itemsAdded: Int,
        val itemsUpdated: Int,
        // When the sync finished. The event flow replays buffered events to new collectors;
        // the auto-download listener uses this to ignore syncs from before it was armed.
        val emittedAt: Long = System.currentTimeMillis()
    ) : TaskEvent()

    // Auto-download events
    data class AutoDownloadStarted(val taskId: String, val bookCount: Int) : TaskEvent()
    data class AutoDownloadComplete(val taskId: String, val downloadedCount: Int) : TaskEvent()

    // System events
    data class WifiAvailable(val available: Boolean) : TaskEvent()
    data class StorageLow(val availableBytes: Long) : TaskEvent()
}
