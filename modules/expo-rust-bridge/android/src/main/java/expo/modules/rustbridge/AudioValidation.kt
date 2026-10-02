package expo.modules.rustbridge

import android.content.Context
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger

private const val TAG = "AudioValidation"

/** More decode errors than this and the file is corrupt; stop decoding early. */
private const val MAX_ERRORS_BEFORE_ABORT = 50

/** Result of the post-conversion corruption check. */
data class AudioValidationResult(
    val isValid: Boolean,
    val errorCount: Int,
    val errorMessage: String,
    val duration: Double,
)

/**
 * Validate a decoded audiobook by decoding it end to end and counting FFmpeg errors.
 * Shared by the foreground (DownloadOrchestrator) and background (DownloadWorker)
 * pipelines; each supplies its own progress sink and cancel check so the behaviour is
 * identical apart from where progress is reported.
 *
 * The "validation_level" preference is "off" to skip validation; any other value
 * (including the retired "full" / "quick") validates. Progress + ETA come from FFmpeg's
 * statistics.
 */
suspend fun validateAudioFile(
    context: Context,
    filePath: String,
    isCancelled: () -> Boolean = { false },
    onProgress: (pct: Int, etaSec: Int) -> Unit = { _, _ -> },
): AudioValidationResult = withContext(Dispatchers.IO) {
    try {
        Log.d(TAG, "Validating audio file: $filePath")

        val probeSession = com.arthenica.ffmpegkit.FFprobeKit.getMediaInformation(filePath)
        val duration = probeSession.mediaInformation?.duration?.toDoubleOrNull() ?: 0.0
        if (duration <= 0) {
            Log.e(TAG, "Invalid duration: $duration")
            return@withContext AudioValidationResult(false, -1, "Could not determine file duration", 0.0)
        }
        Log.d(TAG, "File duration: ${duration}s (${duration / 3600}h)")

        val validationLevel = context.getSharedPreferences("app_settings", Context.MODE_PRIVATE)
            .getString("validation_level", "full") ?: "full"
        if (validationLevel == "off") {
            Log.d(TAG, "Validation skipped (setting=off)")
            onProgress(100, 0)
            return@withContext AudioValidationResult(true, 0, "Validation skipped by setting", duration)
        }

        // Single full-decode pass — NOT per-point seeking. On FFmpegKit's ffmpeg build an
        // input -ss seek into these files traverses to EOF regardless of -t or -frames:a
        // caps (verified on-device: a sample at 0:30 of a 32h book took 184s and the sample
        // times decreased toward the end — cost ∝ distance-to-EOF, i.e. the input is drained
        // past the requested window and no output-side cap stops it). Five seeks therefore
        // cost ~2.5x a single pass AND miss everything between the points. One straight
        // decode is a single EOF traversal that checks every second.
        Log.d(TAG, "Full-decode validation pass (${"%.2f".format(duration / 3600)}h)")

        val errorCounter = AtomicInteger(0)
        val tooManyErrors = AtomicBoolean(false)
        val command = "-v error -i \"$filePath\" -f null -"
        val latch = java.util.concurrent.CountDownLatch(1)

        var lastPct = -1
        val statsCallback = com.arthenica.ffmpegkit.StatisticsCallback { stat ->
            val processedSec = stat.time.toDouble() / 1000.0
            val pct = ((processedSec / duration).coerceIn(0.0, 1.0) * 100.0).toInt()
            if (pct != lastPct) {
                lastPct = pct
                val speed = stat.speed
                val etaSec = if (speed > 0.0)
                    ((duration - processedSec) / speed).toInt().coerceAtLeast(0) else 0
                onProgress(pct, etaSec)
            }
        }
        // Count decode errors live so a badly corrupt file can be aborted early instead of
        // decoding the whole thing. The callback only raises the flag: it can fire before
        // executeAsync has returned the session id, so the cancel happens in the wait loop.
        val logCallback = com.arthenica.ffmpegkit.LogCallback { log ->
            val msg = log.message ?: ""
            if (msg.contains("Error", ignoreCase = true) || msg.contains("Invalid data", ignoreCase = true)) {
                if (errorCounter.incrementAndGet() > MAX_ERRORS_BEFORE_ABORT) tooManyErrors.set(true)
            }
        }

        val session = com.arthenica.ffmpegkit.FFmpegKit.executeAsync(
            command,
            { _ -> latch.countDown() },
            logCallback,
            statsCallback
        )
        val sessionId = session.sessionId

        try {
            var abortSent = false
            while (!latch.await(300, java.util.concurrent.TimeUnit.MILLISECONDS)) {
                if (isCancelled()) {
                    com.arthenica.ffmpegkit.FFmpegKit.cancel(sessionId)
                    throw kotlinx.coroutines.CancellationException("Validation cancelled by user")
                }
                if (!abortSent && tooManyErrors.get()) {
                    Log.w(TAG, "High error count (${errorCounter.get()}), aborting validation early")
                    com.arthenica.ffmpegkit.FFmpegKit.cancel(sessionId)
                    abortSent = true
                }
            }
        } finally {
            onProgress(100, 0)
        }

        val totalErrors = errorCounter.get()
        val isValid = totalErrors == 0
        val errorMessage = if (isValid) {
            "Audio file validated successfully"
        } else {
            "Audio corruption detected: $totalErrors total errors"
        }
        Log.d(TAG, "Validation result: ${if (isValid) "VALID" else "CORRUPT"} ($totalErrors errors)")

        AudioValidationResult(isValid, totalErrors, errorMessage, duration)
    } catch (e: kotlinx.coroutines.CancellationException) {
        // A user cancel must propagate as a cancel, not masquerade as a corrupt file —
        // callers delete the cached source on a failed validation.
        throw e
    } catch (e: Exception) {
        Log.e(TAG, "Error validating audio file", e)
        AudioValidationResult(false, -1, "Validation failed: ${e.message}", 0.0)
    }
}
