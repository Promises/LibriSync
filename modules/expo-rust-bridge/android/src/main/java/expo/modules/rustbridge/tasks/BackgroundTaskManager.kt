package expo.modules.rustbridge.tasks

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.util.Log
import expo.modules.rustbridge.AppPaths
import expo.modules.rustbridge.StageProgressStore
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import java.util.PriorityQueue
import java.util.concurrent.ConcurrentHashMap

/**
 * Central coordinator for all background tasks
 *
 * Responsibilities:
 * - Manages task queue and execution
 * - Coordinates between different workers
 * - Handles task priorities and conflicts
 * - Provides unified event bus for UI updates
 * - Persists task state across app restarts
 *
 * This is a singleton - use getInstance() to access
 */
class BackgroundTaskManager private constructor(
    private val context: Context
) {
    companion object {
        private const val TAG = "BackgroundTaskManager"
        private const val MAX_CONCURRENT_DOWNLOADS = 3

        @Volatile
        private var instance: BackgroundTaskManager? = null

        fun getInstance(context: Context): BackgroundTaskManager {
            return instance ?: synchronized(this) {
                instance ?: BackgroundTaskManager(context.applicationContext).also {
                    instance = it
                }
            }
        }
    }

    // Coroutine scope for all background work (recreated on each start)
    private var scope = CoroutineScope(Dispatchers.Default + SupervisorJob())

    // Event bus for task updates
    private val _eventFlow = MutableSharedFlow<TaskEvent>(replay = 10)
    val eventFlow: SharedFlow<TaskEvent> = _eventFlow.asSharedFlow()

    // Task storage
    private val taskQueue = PriorityQueue<Task>()
    private val activeTasks = ConcurrentHashMap<String, Task>()
    private val taskHistory = ConcurrentHashMap<String, Task>()

    // Workers
    private lateinit var downloadWorker: DownloadWorker
    // REMOVED: Now using WorkManager for periodic tasks
    // private lateinit var tokenRefreshWorker: TokenRefreshWorker
    // private lateinit var librarySyncWorker: LibrarySyncWorker
    private lateinit var autoDownloadWorker: AutoDownloadWorker

    // State
    @Volatile private var isStarted = false
    @Volatile private var isWifiAvailable = false
    private var eventLoopJob: Job? = null

    init {
        Log.d(TAG, "BackgroundTaskManager initialized")
    }

    /**
     * Start the task manager
     * Must be called once when service starts
     */
    fun start() {
        if (isStarted) {
            Log.w(TAG, "Already started")
            return
        }

        Log.d(TAG, "Starting BackgroundTaskManager")
        isStarted = true

        // Recreate scope (in case it was cancelled from previous stop)
        scope = CoroutineScope(Dispatchers.Default + SupervisorJob())
        Log.d(TAG, "Coroutine scope recreated")

        // Initialize workers
        downloadWorker = DownloadWorker(context, this)
        // tokenRefreshWorker = TokenRefreshWorker(context, this) // REMOVED: Now using WorkManager
        // librarySyncWorker = LibrarySyncWorker(context, this)   // REMOVED: Now using WorkManager
        autoDownloadWorker = AutoDownloadWorker(context, this)
        // If auto-download was left enabled in a previous session, re-arm its sync listener —
        // enable() is otherwise only called from the UI, so after a cold start nothing would
        // be listening for LibrarySyncComplete and a sync would never trigger a download.
        autoDownloadWorker.resumeIfEnabled()

        // Start event loop
        startEventLoop()

        // Restore pending tasks from database
        restorePendingTasks()

        // OLD: Start automatic workers - REMOVED, now using Android WorkManager
        // tokenRefreshWorker.start()
        // librarySyncWorker.startAutoSync()
        // These periodic tasks are now handled by:
        // - expo.modules.rustbridge.workers.TokenRefreshWorker (WorkManager)
        // - expo.modules.rustbridge.workers.LibrarySyncWorker (WorkManager)

        Log.d(TAG, "BackgroundTaskManager started successfully (periodic tasks handled by WorkManager)")
    }

    /**
     * Stop the task manager
     */
    fun stop() {
        Log.d(TAG, "Stopping BackgroundTaskManager")
        isStarted = false

        // OLD: Stop monitoring loops - REMOVED, now using WorkManager
        // if (::tokenRefreshWorker.isInitialized) {
        //     tokenRefreshWorker.stop()
        // }
        // if (::librarySyncWorker.isInitialized) {
        //     librarySyncWorker.stopAutoSync()
        // }

        // Stop event loop
        eventLoopJob?.cancel()
        scope.cancel()

        Log.d(TAG, "Download workers stopped (periodic tasks handled by WorkManager)")
    }

    // ========================================================================
    // Public API - Task Management
    // ========================================================================

    /**
     * Enqueue a download task
     */
    fun enqueueDownload(
        asin: String,
        title: String,
        author: String? = null,
        accountJson: String,
        outputDirectory: String,
        quality: String = "High"
    ): String {
        Log.d(TAG, "Enqueueing download: $asin - $title")

        val taskId = "download_$asin"
        // Idempotent: a book already downloading or waiting in the queue must not be stacked
        // a second time, e.g. when successive sync-complete checks pick the same book.
        val alreadyQueued = synchronized(taskQueue) { taskQueue.any { it.id == taskId } }
        if (activeTasks.containsKey(taskId) || alreadyQueued) {
            Log.d(TAG, "Download already active or queued for $asin; skipping duplicate enqueue")
            return taskId
        }

        val task = Task(
            id = taskId,
            type = TaskType.DOWNLOAD,
            priority = TaskPriority.HIGH,
            status = TaskStatus.PENDING,
            metadata = mutableMapOf<String, Any>(
                DownloadTaskMetadata.ASIN to asin,
                DownloadTaskMetadata.TITLE to title,
                "account_json" to accountJson,
                DownloadTaskMetadata.OUTPUT_DIR to outputDirectory,
                "quality" to quality
            ).apply {
                author?.let { put(DownloadTaskMetadata.AUTHOR, it) }
            }
        )

        enqueueTask(task)
        return taskId
    }

    /**
     * Start a library sync
     */
    fun startLibrarySync(fullSync: Boolean = false): String {
        Log.d(TAG, "Starting library sync (full=$fullSync)")

        // Check if already syncing
        if (activeTasks.values.any { it.type == TaskType.LIBRARY_SYNC }) {
            Log.w(TAG, "Library sync already in progress")
            throw IllegalStateException("Library sync already in progress")
        }

        val taskId = "sync_${System.currentTimeMillis()}"
        val task = Task(
            id = taskId,
            type = TaskType.LIBRARY_SYNC,
            priority = TaskPriority.LOW,
            status = TaskStatus.PENDING,
            metadata = mutableMapOf(
                LibrarySyncMetadata.FULL_SYNC to fullSync
            )
        )

        enqueueTask(task)
        return taskId
    }

    /** Whether the task manager (and thus background work) is currently running. */
    fun isRunning(): Boolean = isStarted

    /** Whether auto-download is enabled. Reads the pref directly so it works even before start(). */
    fun isAutoDownloadEnabled(): Boolean =
        context.getSharedPreferences("auto_download_prefs", Context.MODE_PRIVATE)
            .getBoolean("enabled", false)

    /**
     * Enable automatic downloads
     */
    fun enableAutoDownload() {
        // The workers are created in start(); enabling from a stopped service must not
        // crash on the lateinit. start() is idempotent, and it also starts the event loop
        // that relays LibrarySyncComplete to the auto-download listener.
        if (!isStarted) start()
        Log.d(TAG, "Enabling auto-download")
        autoDownloadWorker.enable()
    }

    /**
     * Disable automatic downloads
     */
    fun disableAutoDownload() {
        if (!isStarted) start()
        Log.d(TAG, "Disabling auto-download")
        autoDownloadWorker.disable()
    }

    /**
     * Signal that a library sync finished. This is the trigger the auto-download worker
     * listens for (LibrarySyncComplete was defined and listened for but never emitted, so
     * auto-download never ran after a sync). If the manager isn't started yet, it is started
     * on demand when auto-download is enabled; otherwise this is a no-op.
     */
    fun notifyLibrarySyncComplete(totalItems: Int = 0, itemsAdded: Int = 0, itemsUpdated: Int = 0) {
        if (!isStarted) {
            // A sync can finish before anything started the manager (e.g. the service never
            // came up). If auto-download is on, start now so the listener is armed; otherwise
            // there's nothing to notify.
            val autoOn = context.getSharedPreferences("auto_download_prefs", Context.MODE_PRIVATE)
                .getBoolean("enabled", false)
            if (!autoOn) return
            start()
        }
        scope.launch {
            emitEvent(
                TaskEvent.LibrarySyncComplete(
                    taskId = "library_sync_${System.currentTimeMillis()}",
                    totalItems = totalItems,
                    itemsAdded = itemsAdded,
                    itemsUpdated = itemsUpdated
                )
            )
            Log.d(TAG, "Emitted LibrarySyncComplete → auto-download check")
        }
    }

    /**
     * Pause a task
     */
    suspend fun pauseTask(taskId: String): Boolean {
        Log.d(TAG, "Pausing task: $taskId")

        val task = activeTasks[taskId] ?: run {
            Log.w(TAG, "Task not found: $taskId")
            return false
        }

        return when (task.type) {
            TaskType.DOWNLOAD -> downloadWorker.pause(taskId)
            else -> {
                Log.w(TAG, "Cannot pause task type: ${task.type}")
                false
            }
        }
    }

    /**
     * Resume a task
     */
    suspend fun resumeTask(taskId: String): Boolean {
        Log.d(TAG, "Resuming task: $taskId")

        val task = activeTasks[taskId] ?: taskHistory[taskId] ?: run {
            Log.w(TAG, "Task not found: $taskId")
            return false
        }

        return when (task.type) {
            TaskType.DOWNLOAD -> downloadWorker.resume(taskId)
            else -> {
                Log.w(TAG, "Cannot resume task type: ${task.type}")
                false
            }
        }
    }

    /**
     * Cancel a task
     */
    suspend fun cancelTask(taskId: String): Boolean {
        Log.d(TAG, "Cancelling task: $taskId")

        val task = activeTasks[taskId] ?: run {
            Log.w(TAG, "Task not found: $taskId")
            return false
        }

        return when (task.type) {
            TaskType.DOWNLOAD -> downloadWorker.cancel(taskId)
            else -> {
                task.status = TaskStatus.CANCELLED
                activeTasks.remove(taskId)
                emitEvent(TaskEvent.TaskCancelled(task))
                true
            }
        }
    }

    /**
     * Get all active tasks
     */
    fun getActiveTasks(): List<Task> = activeTasks.values.toList()

    /**
     * Get task by ID
     */
    fun getTask(taskId: String): Task? = activeTasks[taskId] ?: taskHistory[taskId]

    /**
     * Clear all tasks (for debugging/recovery from stuck states)
     */
    fun clearAllTasks() {
        Log.d(TAG, "Clearing all tasks")

        // Stop the download worker's monitoring loops and abort any in-flight conversion so a
        // cleared task actually stops (otherwise it keeps polling/logging forever — the
        // "residual undeletable task" symptom). Guarded: the worker only exists once started.
        if (isStarted) {
            runCatching { downloadWorker.cancelAllMonitoring() }
                .onFailure { Log.w(TAG, "cancelAllMonitoring failed", it) }
        }

        // Cancel all active tasks
        activeTasks.keys.toList().forEach { taskId ->
            try {
                val task = activeTasks[taskId]
                if (task != null) {
                    task.status = TaskStatus.CANCELLED
                    activeTasks.remove(taskId)
                }
            } catch (e: Exception) {
                Log.e(TAG, "Error cancelling task $taskId", e)
            }
        }

        // Clear queue
        synchronized(taskQueue) {
            taskQueue.clear()
        }

        // Clear history
        taskHistory.clear()

        Log.d(TAG, "All tasks cleared")
    }

    // ========================================================================
    // Internal API - For Workers
    // ========================================================================

    /**
     * Emit an event to the event bus
     */
    suspend fun emitEvent(event: TaskEvent) {
        _eventFlow.emit(event)
    }

    /**
     * Register a task as active
     */
    fun registerActiveTask(task: Task) {
        activeTasks[task.id] = task
        Log.d(TAG, "Registered active task: ${task.id} (${task.type})")
    }

    /**
     * Unregister an active task (move to history)
     */
    fun unregisterActiveTask(taskId: String) {
        activeTasks.remove(taskId)?.let { task ->
            taskHistory[taskId] = task
            task.getMetadataString(DownloadTaskMetadata.ASIN)?.let { StageProgressStore.clear(it) }
            Log.d(TAG, "Unregistered active task: $taskId")
        }
    }

    /**
     * Update task metadata
     */
    fun updateTaskMetadata(taskId: String, updates: Map<String, Any>) {
        activeTasks[taskId]?.let { task ->
            task.metadata.putAll(updates)
            // Mirror download-pipeline stage progress to the JS-visible store so the
            // in-app library list shows auto-download stage % + ETA, not just the notification.
            val asin = task.getMetadataString(DownloadTaskMetadata.ASIN)
            val stage = task.getMetadataString(DownloadTaskMetadata.STAGE)
            if (asin != null && stage != null) {
                StageProgressStore.update(
                    asin,
                    stage,
                    task.getMetadataInt(DownloadTaskMetadata.PERCENTAGE) ?: 0,
                    (task.getMetadataInt(DownloadTaskMetadata.ETA_SECONDS) ?: 0).toLong()
                )
            }
        }
    }

    /**
     * Get database path
     */
    fun getDbPath(): String {
        return AppPaths.databasePath(context)
    }

    /**
     * Check if WiFi (or Ethernet) is available.
     *
     * Queried live from ConnectivityManager. The old design relied on
     * BackgroundTaskService pushing status via setWifiAvailable(), but that service is a
     * no-op now (WorkManager owns periodic work), so the pushed flag stayed false and the
     * auto-download WiFi gate always failed. Falls back to the pushed flag if the system
     * service is somehow unavailable.
     */
    fun isWifiAvailable(): Boolean {
        val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
            ?: return isWifiAvailable
        val caps = cm.getNetworkCapabilities(cm.activeNetwork) ?: return false
        return caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) ||
            caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)
    }

    /**
     * Update WiFi availability
     */
    fun setWifiAvailable(available: Boolean) {
        if (isWifiAvailable != available) {
            isWifiAvailable = available
            Log.d(TAG, "WiFi availability changed: $available")
            scope.launch {
                emitEvent(TaskEvent.WifiAvailable(available))
            }
        }
    }

    // ========================================================================
    // Private Methods
    // ========================================================================

    /**
     * Enqueue a task for execution
     */
    private fun enqueueTask(task: Task) {
        synchronized(taskQueue) {
            taskQueue.offer(task)
            Log.d(TAG, "Task enqueued: ${task.id} (priority=${task.priority}, type=${task.type})")
        }
    }

    /**
     * Start the event loop that processes tasks
     */
    private fun startEventLoop() {
        Log.d(TAG, "startEventLoop() called")
        try {
            eventLoopJob = scope.launch {
                Log.d(TAG, "Event loop coroutine started")

                while (isActive) {
                    try {
                        // Process pending tasks
                        processNextTask()

                        // Sleep briefly to avoid busy-waiting
                        delay(1000)
                    } catch (e: Exception) {
                        Log.e(TAG, "Error in event loop", e)
                    }
                }

                Log.d(TAG, "Event loop stopped")
            }
            Log.d(TAG, "Event loop job created successfully")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to start event loop", e)
        }
    }

    /**
     * Process the next task in the queue
     */
    private suspend fun processNextTask() {
        val task = synchronized(taskQueue) {
            val t = taskQueue.poll()
            if (t != null) {
                Log.d(TAG, "Polled task from queue: ${t.id} (${t.type})")
            }
            t
        } ?: return

        // Check if we can start this task
        if (!canStartTask(task)) {
            Log.d(TAG, "Cannot start task yet: ${task.id}, re-queueing")
            // Re-queue for later
            synchronized(taskQueue) {
                taskQueue.offer(task)
            }
            return
        }

        // Mark as running
        task.status = TaskStatus.RUNNING
        task.startedAt = java.util.Date()
        registerActiveTask(task)

        // Emit started event
        emitEvent(TaskEvent.TaskStarted(task))

        Log.d(TAG, "Starting task: ${task.id} (${task.type})")

        // Dispatch to appropriate worker
        scope.launch {
            try {
                when (task.type) {
                    TaskType.DOWNLOAD -> downloadWorker.execute(task)
                    // REMOVED: These task types are now handled by WorkManager
                    // TaskType.TOKEN_REFRESH -> tokenRefreshWorker.execute(task)
                    // TaskType.LIBRARY_SYNC -> librarySyncWorker.execute(task)
                    TaskType.TOKEN_REFRESH -> {
                        Log.w(TAG, "TOKEN_REFRESH tasks are now handled by WorkManager, ignoring")
                        task.status = TaskStatus.FAILED
                        task.error = "Token refresh is now handled by WorkManager. Use Settings to configure."
                        emitEvent(TaskEvent.TaskFailed(task, task.error!!))
                        unregisterActiveTask(task.id)
                    }
                    TaskType.LIBRARY_SYNC -> {
                        Log.w(TAG, "LIBRARY_SYNC tasks are now handled by WorkManager, ignoring")
                        task.status = TaskStatus.FAILED
                        task.error = "Library sync is now handled by WorkManager. Use Settings to configure."
                        emitEvent(TaskEvent.TaskFailed(task, task.error!!))
                        unregisterActiveTask(task.id)
                    }
                    TaskType.AUTO_DOWNLOAD -> autoDownloadWorker.execute(task)
                }
            } catch (e: Exception) {
                Log.e(TAG, "Task execution failed: ${task.id}", e)
                task.status = TaskStatus.FAILED
                task.error = e.message
                task.completedAt = java.util.Date()
                emitEvent(TaskEvent.TaskFailed(task, e.message ?: "Unknown error"))
                unregisterActiveTask(task.id)
            }
        }
    }

    /**
     * Check if a task can be started based on current state
     */
    private fun canStartTask(task: Task): Boolean {
        return when (task.type) {
            TaskType.DOWNLOAD -> {
                val activeDownloads = activeTasks.values.count { it.type == TaskType.DOWNLOAD }
                activeDownloads < MAX_CONCURRENT_DOWNLOADS
            }
            TaskType.LIBRARY_SYNC -> {
                // Only one sync at a time
                activeTasks.values.none { it.type == TaskType.LIBRARY_SYNC }
            }
            TaskType.TOKEN_REFRESH -> {
                // Token refresh always runs
                true
            }
            TaskType.AUTO_DOWNLOAD -> {
                // Auto-download requires WiFi (queried live)
                isWifiAvailable()
            }
        }
    }

    /**
     * Restore pending tasks from database on startup
     */
    private fun restorePendingTasks() {
        scope.launch {
            try {
                Log.d(TAG, "Restoring pending tasks from database")

                // Restore download tasks
                downloadWorker.restorePendingTasks()

                Log.d(TAG, "Pending tasks restored")
            } catch (e: Exception) {
                Log.e(TAG, "Failed to restore pending tasks", e)
            }
        }
    }
}
