package expo.modules.rustbridge.workers

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.Build
import android.util.Log
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters
import expo.modules.rustbridge.AppPaths
import expo.modules.rustbridge.ExpoRustBridgeModule
import expo.modules.rustbridge.ExistingDownloadScanner
import expo.modules.rustbridge.tasks.BackgroundTaskManager
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.util.Date

/**
 * WorkManager-based library sync worker
 *
 * Features:
 * - Syncs library from Audible API page-by-page
 * - Refreshes token if expired before syncing
 * - Respects network constraints (WiFi-only if configured)
 * - Handles errors gracefully with retry logic
 */
class LibrarySyncWorker(
    context: Context,
    params: WorkerParameters
) : CoroutineWorker(context, params) {

    companion object {
        private const val TAG = "LibrarySyncWorker"

        /**
         * Parse date from multiple possible formats
         */
        private fun parseDate(dateStr: String): Date? {
            // Try ISO 8601 format first (e.g., "2025-10-24T00:09:28.000Z")
            try {
                return java.text.SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss.SSS'Z'", java.util.Locale.US).apply {
                    timeZone = java.util.TimeZone.getTimeZone("UTC")
                }.parse(dateStr)
            } catch (e: Exception) {
                // Ignore, try next format
            }

            // Try Java toString() format (e.g., "Fri Oct 24 00:09:28 GMT+02:00 2025")
            try {
                return java.text.SimpleDateFormat("EEE MMM dd HH:mm:ss zzz yyyy", java.util.Locale.US).parse(dateStr)
            } catch (e: Exception) {
                // Ignore, try next format
            }

            // Try ISO 8601 without milliseconds (e.g., "2025-10-24T00:09:28Z")
            try {
                return java.text.SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss'Z'", java.util.Locale.US).apply {
                    timeZone = java.util.TimeZone.getTimeZone("UTC")
                }.parse(dateStr)
            } catch (e: Exception) {
                // Ignore
            }

            return null
        }
    }

    override suspend fun doWork(): Result = withContext(Dispatchers.IO) {
        try {
            Log.d(TAG, "Library sync worker started")

            // Get database path
            val dbPath = AppPaths.databasePath(applicationContext)

            // Load account from SQLite database
            val getAccountParams = JSONObject().apply {
                put("db_path", dbPath)
            }
            val accountResultJson = ExpoRustBridgeModule.nativeGetPrimaryAccount(getAccountParams.toString())
            val accountResultObj = JSONObject(accountResultJson)

            if (!accountResultObj.getBoolean("success")) {
                Log.d(TAG, "No account found, skipping library sync")
                return@withContext Result.failure()
            }

            var accountJson = accountResultObj.getJSONObject("data").optString("account")
            if (accountJson.isNullOrEmpty() || accountJson == "null") {
                Log.d(TAG, "No account in database, skipping library sync")
                return@withContext Result.failure()
            }

            var account = JSONObject(accountJson)

            // Check if token is expired or expiring soon, and refresh if needed
            val refreshedAccount = checkAndRefreshTokenIfNeeded(account, dbPath)
            if (refreshedAccount != null) {
                account = refreshedAccount
                accountJson = account.toString()
                Log.d(TAG, "Token refreshed before starting sync")
            }

            // Initialize database
            val initParams = JSONObject().apply {
                put("db_path", dbPath)
            }
            ExpoRustBridgeModule.nativeInitDatabase(initParams.toString())

            // Sync library page-by-page (Audible API uses 1-based page numbers)
            var page = 1
            var hasMore = true
            var totalItemsSynced = 0
            var totalItemsAdded = 0
            var totalItemsUpdated = 0

            while (hasMore) {
                Log.d(TAG, "Syncing page $page...")

                // Sync this page
                val syncParams = JSONObject().apply {
                    put("db_path", dbPath)
                    put("account_json", accountJson)
                    put("page", page)
                }
                val pageResultJson = ExpoRustBridgeModule.nativeSyncLibraryPage(syncParams.toString())
                val pageResultObj = JSONObject(pageResultJson)

                if (!pageResultObj.getBoolean("success")) {
                    val error = pageResultObj.optString("error", "Sync failed")
                    Log.e(TAG, "Library sync failed on page $page: $error")
                    return@withContext Result.retry()
                }

                val statsObj = pageResultObj.getJSONObject("data")
                val totalItems = statsObj.getInt("total_items")
                val booksAdded = statsObj.getInt("books_added")
                val booksUpdated = statsObj.getInt("books_updated")
                hasMore = statsObj.getBoolean("has_more")

                // Update totals
                totalItemsSynced = totalItems
                totalItemsAdded += booksAdded
                totalItemsUpdated += booksUpdated

                Log.d(TAG, "Page $page synced: $totalItems total, $booksAdded added, $booksUpdated updated, hasMore=$hasMore")

                page++
            }

            Log.d(TAG, "Library sync complete: $totalItemsSynced items ($totalItemsAdded added, $totalItemsUpdated updated)")

            val downloadDirectory = ExistingDownloadScanner.getSavedDownloadDirectory(applicationContext)
            if (!downloadDirectory.isNullOrBlank()) {
                try {
                    val scanResult = ExistingDownloadScanner.scan(applicationContext, dbPath, downloadDirectory)
                    Log.d(
                        TAG,
                        "Existing download scan complete: ${scanResult.booksLinked} linked, " +
                            "${scanResult.booksAlreadyLinked} already linked, ${scanResult.filesUnmatched} unmatched"
                    )
                } catch (e: Exception) {
                    Log.w(TAG, "Existing download scan failed after successful library sync", e)
                }
            } else {
                Log.d(TAG, "No download directory configured; skipping existing download scan")
            }

            // Let auto-download react to the freshly synced library. This is the WorkManager
            // sync path (debug "Start Library Sync" and scheduled auto-sync); it goes through
            // nativeSyncLibraryPage directly, so it must notify the manager itself — no-op
            // unless auto-download is enabled. Runs after the existing-download scan so books
            // already present on disk are linked (and thus skipped) first.
            try {
                BackgroundTaskManager.getInstance(applicationContext)
                    .notifyLibrarySyncComplete(totalItemsSynced, totalItemsAdded, totalItemsUpdated)
            } catch (e: Exception) {
                Log.w(TAG, "Failed to notify auto-download after library sync", e)
            }

            return@withContext Result.success()

        } catch (e: Exception) {
            Log.e(TAG, "Library sync worker failed", e)
            return@withContext Result.retry()
        }
    }

    /**
     * Check if token needs refresh and refresh it if needed
     * Returns updated account JSON if token was refreshed, null otherwise
     */
    private suspend fun checkAndRefreshTokenIfNeeded(account: JSONObject, dbPath: String): JSONObject? = withContext(Dispatchers.IO) {
        try {
            val identity = account.optJSONObject("identity") ?: return@withContext null

            // Get token expiry
            val accessTokenObj = identity.opt("access_token")
            val expiresAtStr = when (accessTokenObj) {
                is JSONObject -> accessTokenObj.optString("expires_at")
                else -> null
            } ?: return@withContext null

            // Parse expiry date (supports multiple formats)
            val expiresAt = parseDate(expiresAtStr)
            if (expiresAt == null) {
                Log.e(TAG, "Invalid expiry date: $expiresAtStr")
                return@withContext null
            }

            val now = Date()
            val timeRemaining = expiresAt.time - now.time

            Log.d(TAG, "Token check: expires at $expiresAt (${timeRemaining / 1000 / 60} minutes remaining)")

            // Refresh if expired or expiring within 5 minutes
            if (timeRemaining <= 0 || timeRemaining < 300_000) {
                Log.d(TAG, "Token expired or expiring soon - refreshing before sync")

                val countryCode = account.getJSONObject("locale").getString("country_code")
                val refreshToken = identity.getString("refresh_token")
                val deviceSerial = identity.getString("device_serial_number")

                // Call token refresh
                val params = JSONObject().apply {
                    put("locale_code", countryCode)
                    put("refresh_token", refreshToken)
                    put("device_serial", deviceSerial)
                }
                val resultJson = ExpoRustBridgeModule.nativeRefreshAccessToken(params.toString())
                val resultObj = JSONObject(resultJson)

                if (!resultObj.getBoolean("success")) {
                    throw Exception("Token refresh failed: ${resultObj.optString("error")}")
                }

                // Update account with new token
                val dataObj = resultObj.getJSONObject("data")
                val newAccessToken = dataObj.getString("access_token")
                val expiresIn = dataObj.optString("expires_in", "3600").toLongOrNull() ?: 3600L
                val newExpiry = Date(System.currentTimeMillis() + (expiresIn * 1000))

                identity.put("access_token", JSONObject().apply {
                    put("token", newAccessToken)
                    put("expires_at", java.text.SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss.SSS'Z'", java.util.Locale.US).apply {
                        timeZone = java.util.TimeZone.getTimeZone("UTC")
                    }.format(newExpiry))
                })

                // Only update refresh token if Amazon actually returned a new one
                if (dataObj.has("refresh_token") && !dataObj.isNull("refresh_token")) {
                    val newRefreshToken = dataObj.getString("refresh_token")
                    if (newRefreshToken.isNotEmpty()) {
                        identity.put("refresh_token", newRefreshToken)
                        Log.d(TAG, "Updated refresh_token from Amazon response")
                    } else {
                        Log.d(TAG, "Amazon returned empty refresh_token, keeping existing one")
                    }
                } else {
                    Log.d(TAG, "Amazon didn't return refresh_token, keeping existing one")
                }

                // Save updated account to database
                val saveParams = JSONObject().apply {
                    put("db_path", dbPath)
                    put("account_json", account.toString())
                }
                val saveResult = ExpoRustBridgeModule.nativeSaveAccount(saveParams.toString())
                val saveResultObj = JSONObject(saveResult)

                if (!saveResultObj.getBoolean("success")) {
                    Log.e(TAG, "Failed to save refreshed token: ${saveResultObj.optString("error")}")
                }

                Log.d(TAG, "Token refreshed successfully. New expiry: $newExpiry")
                return@withContext account
            }

            return@withContext null
        } catch (e: Exception) {
            Log.e(TAG, "Error checking/refreshing token", e)
            // Don't throw - allow sync to proceed and fail naturally if token is invalid
            return@withContext null
        }
    }
}
