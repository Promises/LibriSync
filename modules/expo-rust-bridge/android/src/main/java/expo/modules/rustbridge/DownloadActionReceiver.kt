package expo.modules.rustbridge

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log
import org.json.JSONArray
import org.json.JSONObject

/**
 * BroadcastReceiver for handling notification action buttons
 *
 * Handles:
 * - Pause download button
 * - Cancel download button
 */
class DownloadActionReceiver : BroadcastReceiver() {
    companion object {
        private const val TAG = "DownloadActionReceiver"
        const val ACTION_PAUSE = "expo.modules.rustbridge.PAUSE_DOWNLOAD"
        const val ACTION_RESUME = "expo.modules.rustbridge.RESUME_DOWNLOAD"
        const val ACTION_CANCEL = "expo.modules.rustbridge.CANCEL_DOWNLOAD"
        const val ACTION_RETRY = "expo.modules.rustbridge.RETRY_DOWNLOAD"
    }

    override fun onReceive(context: Context, intent: Intent) {
        Log.d(TAG, "onReceive called with action: ${intent.action}")

        val asin = intent.getStringExtra("asin")
        if (asin == null) {
            Log.e(TAG, "No ASIN provided in intent")
            return
        }

        val dbPath = AppPaths.databasePath(context)

        Log.d(TAG, "Received action: ${intent.action} for ASIN: $asin")

        when (intent.action) {
            ACTION_PAUSE -> {
                Log.d(TAG, "Handling pause action")
                handlePause(context, dbPath, asin)
            }
            ACTION_RESUME -> {
                Log.d(TAG, "Handling resume action")
                handleResume(context, dbPath, asin)
            }
            ACTION_CANCEL -> {
                Log.d(TAG, "Handling cancel action")
                handleCancel(context, dbPath, asin)
            }
            ACTION_RETRY -> {
                Log.d(TAG, "Handling retry action")
                handleRetry(context, dbPath, asin)
            }
            else -> {
                Log.w(TAG, "Unknown action: ${intent.action}")
            }
        }
    }

    private fun handlePause(context: Context, dbPath: String, asin: String) {
        try {
            // Find the task ID for this ASIN
            val listParams = JSONObject().apply {
                put("db_path", dbPath)
            }

            val listResult = ExpoRustBridgeModule.nativeListDownloadTasks(listParams.toString())
            val parsed = parseJsonResponse(listResult)

            Log.d(TAG, "List tasks result: $parsed")

            if (parsed["success"] == true) {
                val data = parsed["data"] as? Map<*, *>
                @Suppress("UNCHECKED_CAST")
                val tasks = data?.get("tasks") as? List<Map<*, *>> ?: emptyList()

                Log.d(TAG, "Found ${tasks.size} tasks")
                tasks.forEach { task ->
                    Log.d(TAG, "Task: asin=${task["asin"]}, status=${task["status"]}, task_id=${task["task_id"]}")
                }

                val task = tasks.find { it["asin"] == asin }
                val taskId = task?.get("task_id") as? String

                Log.d(TAG, "Looking for ASIN: $asin, found task: $task")

                if (taskId != null) {
                    // Pause the download
                    val pauseParams = JSONObject().apply {
                        put("db_path", dbPath)
                        put("task_id", taskId)
                    }

                    val pauseResult = ExpoRustBridgeModule.nativePauseDownload(pauseParams.toString())
                    val pauseParsed = parseJsonResponse(pauseResult)

                    if (pauseParsed["success"] == true) {
                        // Mark as manually paused (so it won't auto-resume on WiFi)
                        markAsManuallyPaused(context, asin)

                        Log.d(TAG, "Successfully paused download: $asin")

                        // Show paused notification
                        val notificationManager = DownloadNotificationManager(context)
                        val title = task["title"] as? String ?: "Audiobook"
                        val bytesDownloaded = (task["bytes_downloaded"] as? Number)?.toLong() ?: 0L
                        val totalBytes = (task["total_bytes"] as? Number)?.toLong() ?: 1L
                        val percentage = ((bytesDownloaded.toDouble() / totalBytes) * 100).toInt()

                        notificationManager.showPaused(asin, title, null, percentage)
                    } else {
                        Log.e(TAG, "Failed to pause: ${pauseParsed["error"]}")
                    }
                } else {
                    Log.e(TAG, "Task not found for ASIN: $asin")
                }
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error handling pause", e)
        }
    }

    /**
     * Mark an ASIN as manually paused (won't auto-resume on WiFi)
     */
    private fun markAsManuallyPaused(context: Context, asin: String) {
        val prefs = context.getSharedPreferences("download_orchestrator_prefs", Context.MODE_PRIVATE)
        val manuallyPaused = prefs.getStringSet("manually_paused_asins", emptySet())?.toMutableSet() ?: mutableSetOf()
        manuallyPaused.add(asin)
        prefs.edit().putStringSet("manually_paused_asins", manuallyPaused).apply()
        Log.d(TAG, "Marked $asin as manually paused")
    }

    /**
     * Clear manual pause marker (when user manually resumes)
     */
    private fun clearManuallyPaused(context: Context, asin: String) {
        val prefs = context.getSharedPreferences("download_orchestrator_prefs", Context.MODE_PRIVATE)
        val manuallyPaused = prefs.getStringSet("manually_paused_asins", emptySet())?.toMutableSet() ?: mutableSetOf()
        if (manuallyPaused.remove(asin)) {
            prefs.edit().putStringSet("manually_paused_asins", manuallyPaused).apply()
            Log.d(TAG, "Cleared manual pause marker for $asin")
        }
    }

    private fun handleResume(context: Context, dbPath: String, asin: String) {
        try {
            // Find the task ID for this ASIN
            val listParams = JSONObject().apply {
                put("db_path", dbPath)
            }

            val listResult = ExpoRustBridgeModule.nativeListDownloadTasks(listParams.toString())
            val parsed = parseJsonResponse(listResult)

            if (parsed["success"] == true) {
                val data = parsed["data"] as? Map<*, *>
                @Suppress("UNCHECKED_CAST")
                val tasks = data?.get("tasks") as? List<Map<*, *>> ?: emptyList()

                val task = tasks.find { it["asin"] == asin }
                val taskId = task?.get("task_id") as? String

                if (taskId != null) {
                    // Resume the download
                    val resumeParams = JSONObject().apply {
                        put("db_path", dbPath)
                        put("task_id", taskId)
                    }

                    val resumeResult = ExpoRustBridgeModule.nativeResumeDownload(resumeParams.toString())
                    val resumeParsed = parseJsonResponse(resumeResult)

                    if (resumeParsed["success"] == true) {
                        // Clear manual pause marker
                        clearManuallyPaused(context, asin)

                        Log.d(TAG, "Successfully resumed download: $asin")

                        // The progress notification will be shown automatically by orchestrator
                        // Just cancel the paused notification
                        val notificationManager = DownloadNotificationManager(context)
                        asin?.let { notificationManager.cancelForAsin(it) }
                    } else {
                        Log.e(TAG, "Failed to resume: ${resumeParsed["error"]}")
                    }
                } else {
                    Log.e(TAG, "Task not found for ASIN: $asin")
                }
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error handling resume", e)
        }
    }

    private fun handleRetry(context: Context, dbPath: String, asin: String) {
        try {
            Log.d(TAG, "Retrying conversion for $asin")
            DownloadService.retryConversion(context, dbPath, asin)
            // Dismiss the failure notification; a progress notification replaces it.
            DownloadNotificationManager(context).cancelForAsin(asin)
        } catch (e: Exception) {
            Log.e(TAG, "Error handling retry", e)
        }
    }

    private fun handleCancel(context: Context, dbPath: String, asin: String) {
        try {
            // Find the task ID for this ASIN
            val listParams = JSONObject().apply {
                put("db_path", dbPath)
            }

            val listResult = ExpoRustBridgeModule.nativeListDownloadTasks(listParams.toString())
            val parsed = parseJsonResponse(listResult)

            if (parsed["success"] == true) {
                val data = parsed["data"] as? Map<*, *>
                @Suppress("UNCHECKED_CAST")
                val tasks = data?.get("tasks") as? List<Map<*, *>> ?: emptyList()

                val task = tasks.find { it["asin"] == asin }
                val taskId = task?.get("task_id") as? String

                // Best-effort: cancel the Rust download if it is still running. Past the
                // download stage (decrypt/validate/copy) the download has already finished,
                // so this can report failure — that is fine, the cleanup below still runs.
                if (taskId != null) {
                    val cancelParams = JSONObject().apply {
                        put("db_path", dbPath)
                        put("task_id", taskId)
                    }
                    val cancelParsed = parseJsonResponse(
                        ExpoRustBridgeModule.nativeCancelDownload(cancelParams.toString())
                    )
                    if (cancelParsed["success"] != true) {
                        Log.w(TAG, "nativeCancelDownload did not cancel $asin (likely past the download stage): ${cancelParsed["error"]}")
                    }
                } else {
                    Log.w(TAG, "No download task for $asin; treating as a conversion/queue cancel")
                }

                // Always stop monitoring — this aborts an in-flight conversion (decrypt /
                // validate / copy) and dequeues a queued book. Previously this was gated on
                // the Rust cancel succeeding, so a mid-copy cancel never reached the abort
                // and the copy ran to completion.
                clearManuallyPaused(context, asin)
                val stopIntent = Intent(context, DownloadService::class.java).apply {
                    action = "expo.modules.rustbridge.STOP_MONITORING"
                    putExtra("asin", asin)
                }
                context.startService(stopIntent)
                DownloadNotificationManager(context).cancelForAsin(asin)
                Log.d(TAG, "Cancelled/cleaned up download: $asin")
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error handling cancel", e)
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
                    dataMap[key] = parseJsonValue(value)
                }

                mapOf("success" to true, "data" to dataMap)
            } else {
                mapOf("success" to false, "error" to json.getString("error"))
            }
        } catch (e: Exception) {
            Log.e(TAG, "Parse error", e)
            mapOf("success" to false, "error" to "Parse error: ${e.message}")
        }
    }

    private fun parseJsonValue(value: Any?): Any? {
        return when (value) {
            is JSONObject -> {
                val map = mutableMapOf<String, Any?>()
                value.keys().forEach { key ->
                    map[key] = parseJsonValue(value.get(key))
                }
                map
            }
            is JSONArray -> {
                (0 until value.length()).map { i ->
                    parseJsonValue(value.get(i))
                }
            }
            JSONObject.NULL -> null
            else -> value
        }
    }
}
