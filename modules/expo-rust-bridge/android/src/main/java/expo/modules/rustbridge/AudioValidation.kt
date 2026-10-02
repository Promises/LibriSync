package expo.modules.rustbridge

import android.content.Context
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong

private const val TAG = "AudioValidation"

/** Result of the post-conversion corruption check. */
data class AudioValidationResult(
    val isValid: Boolean,
    val errorCount: Int,
    val errorMessage: String,
    val duration: Double,
    val samplePoints: List<String> = emptyList()
)

private fun formatTimestamp(seconds: Long): String {
    val hours = seconds / 3600
    val minutes = (seconds % 3600) / 60
    val secs = seconds % 60
    return "%02d:%02d:%02d".format(hours, minutes, secs)
}

/**
 * Validate a decoded audiobook by decoding short samples at several points and counting
 * FFmpeg errors. Shared by the foreground (DownloadOrchestrator) and background
 * (DownloadWorker) pipelines; each supplies its own progress sink and cancel check so the
 * behaviour is identical apart from where progress is reported.
 *
 * Depth is read from the "validation_level" preference: "full" (all points), "quick"
 * (ends only) or "off" (skip). Progress + ETA are driven by a timer because the cost is
 * seeking into a huge file, which emits no FFmpeg statistics.
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
        // decode is a single EOF traversal that checks every second. ("quick" now behaves
        // like "full"; only "off" skips.) Progress + ETA come from FFmpeg's real statistics.
        Log.d(TAG, "Full-decode validation pass (level=$validationLevel, ${"%.2f".format(duration / 3600)}h)")

        val errorCounter = AtomicInteger(0)
        val abortedForErrors = AtomicInteger(0)
        val command = "-v error -i \"$filePath\" -f null -"
        val latch = java.util.concurrent.CountDownLatch(1)
        var sessionId = 0L

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
        // decoding the whole thing.
        val logCallback = com.arthenica.ffmpegkit.LogCallback { log ->
            val msg = log.message ?: ""
            if (msg.contains("Error", ignoreCase = true) || msg.contains("Invalid data", ignoreCase = true)) {
                val n = errorCounter.incrementAndGet()
                if (n > 50 && abortedForErrors.compareAndSet(0, 1)) {
                    Log.w(TAG, "High error count ($n), aborting validation early")
                    if (sessionId != 0L) com.arthenica.ffmpegkit.FFmpegKit.cancel(sessionId)
                }
            }
        }

        val session = com.arthenica.ffmpegkit.FFmpegKit.executeAsync(
            command,
            { _ -> latch.countDown() },
            logCallback,
            statsCallback
        )
        sessionId = session.sessionId

        try {
            while (!latch.await(300, java.util.concurrent.TimeUnit.MILLISECONDS)) {
                if (isCancelled()) {
                    com.arthenica.ffmpegkit.FFmpegKit.cancel(sessionId)
                    throw kotlinx.coroutines.CancellationException("Validation cancelled by user")
                }
            }
        } finally {
            onProgress(100, 0)
        }

        val totalErrors = errorCounter.get()
        val sampleResults = emptyList<String>()

        val isValid = totalErrors == 0
        val errorMessage = if (isValid) {
            "Audio file validated successfully"
        } else {
            "Audio corruption detected: $totalErrors total errors\n${sampleResults.joinToString("\n")}"
        }
        Log.d(TAG, "Validation result: ${if (isValid) "VALID" else "CORRUPT"} ($totalErrors errors)")

        AudioValidationResult(isValid, totalErrors, errorMessage, duration, sampleResults)
    } catch (e: kotlinx.coroutines.CancellationException) {
        // A user cancel must propagate as a cancel, not masquerade as a corrupt file —
        // callers delete the cached source on a failed validation.
        throw e
    } catch (e: Exception) {
        Log.e(TAG, "Error validating audio file", e)
        AudioValidationResult(false, -1, "Validation failed: ${e.message}", 0.0)
    }
}
