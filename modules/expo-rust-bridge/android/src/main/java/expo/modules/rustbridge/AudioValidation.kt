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

        // Check: 30s, 25%, 50%, 75%, end-30s
        val samplePoints = listOf(
            30.0,
            duration * 0.25,
            duration * 0.50,
            duration * 0.75,
            maxOf(duration - 30, 60.0)
        ).distinct().sorted()

        val validationLevel = context.getSharedPreferences("app_settings", Context.MODE_PRIVATE)
            .getString("validation_level", "full") ?: "full"
        if (validationLevel == "off") {
            Log.d(TAG, "Validation skipped (setting=off)")
            onProgress(100, 0)
            return@withContext AudioValidationResult(true, 0, "Validation skipped by setting", duration)
        }
        val effectiveSamplePoints = if (validationLevel == "quick")
            listOf(samplePoints.first(), samplePoints.last()).distinct()
        else samplePoints

        Log.d(TAG, "Sampling ${effectiveSamplePoints.size} points ($validationLevel): ${effectiveSamplePoints.map { "%.1fmin".format(it / 60) }}")

        var totalErrors = 0
        val sampleResults = mutableListOf<String>()
        val totalSamples = effectiveSamplePoints.size
        val testDuration = 10 // seconds decoded per sample

        // Seed a timer-driven estimate BEFORE the first sample finishes, refined by each
        // real sample's measured duration.
        val completedSamples = AtomicInteger(0)
        val sampleStartMs = AtomicLong(System.currentTimeMillis())
        val avgSampleMs = AtomicLong(4000L)

        val progressTicker = launch {
            var lastPct = -1
            while (isActive) {
                val done = completedSamples.get()
                val avg = avgSampleMs.get().toDouble()
                val sampleElapsed = (System.currentTimeMillis() - sampleStartMs.get()).toDouble()
                val subFrac = (sampleElapsed / avg).coerceIn(0.0, 0.99)
                val overall = ((done + subFrac) / totalSamples).coerceIn(0.0, 0.999)
                val pct = (overall * 100.0).toInt()
                if (pct != lastPct) {
                    lastPct = pct
                    val remaining = (totalSamples - (done + subFrac)).coerceAtLeast(0.0)
                    val etaSec = (remaining * avg / 1000.0).toInt().coerceAtLeast(0)
                    onProgress(pct, etaSec)
                }
                delay(400)
            }
        }

        try {
            for ((index, timestamp) in effectiveSamplePoints.withIndex()) {
                if (isCancelled()) throw kotlinx.coroutines.CancellationException("Validation cancelled by user")
                sampleStartMs.set(System.currentTimeMillis())
                val command = "-v error -ss $timestamp -i \"$filePath\" -t $testDuration -f null -"

                val session = com.arthenica.ffmpegkit.FFmpegKit.execute(command)
                val output = session.allLogsAsString

                val errors = output.lines().count {
                    it.contains("Error", ignoreCase = true) ||
                    it.contains("Invalid data", ignoreCase = true)
                }

                totalErrors += errors
                val statusMark = if (errors == 0) "✓" else "✗ $errors errors"
                val timestampStr = formatTimestamp(timestamp.toLong())
                sampleResults.add("  [$timestampStr] $statusMark")

                val took = System.currentTimeMillis() - sampleStartMs.get()
                avgSampleMs.set(
                    if (index == 0) took.coerceAtLeast(250L)
                    else (0.6 * avgSampleMs.get() + 0.4 * took).toLong().coerceAtLeast(250L)
                )
                completedSamples.set(index + 1)

                Log.d(TAG, "Sample ${index + 1}/$totalSamples at $timestampStr: $errors errors (${took}ms)")

                if (errors > 50) {
                    Log.w(TAG, "High error count detected at $timestampStr, stopping validation")
                    break
                }
            }
        } finally {
            progressTicker.cancel()
            onProgress(100, 0)
        }

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
