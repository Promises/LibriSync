package expo.modules.rustbridge

import java.io.InputStream
import java.io.OutputStream

/**
 * Copy [input] to [output] while reporting progress and an ETA.
 *
 * [onProgress] is invoked as (percentage 0..100, etaSeconds) and only fires on
 * whole-percent increases to avoid flooding the notification. ETA is derived from
 * the observed throughput so far. If [totalBytes] <= 0 the copy still runs but no
 * progress is reported (percentage would be meaningless without a known size).
 */
fun copyStreamWithProgress(
    input: InputStream,
    output: OutputStream,
    totalBytes: Long,
    onProgress: (Int, Int) -> Unit
) {
    val buffer = ByteArray(256 * 1024)
    var copied = 0L
    var lastPct = -1
    val startMs = System.currentTimeMillis()

    while (true) {
        val read = input.read(buffer)
        if (read < 0) break
        output.write(buffer, 0, read)
        copied += read

        if (totalBytes > 0) {
            val pct = ((copied.toDouble() / totalBytes) * 100.0).toInt().coerceIn(0, 100)
            if (pct > lastPct) {
                lastPct = pct
                val elapsedSec = (System.currentTimeMillis() - startMs) / 1000.0
                val etaSec = if (elapsedSec > 0.0 && copied > 0) {
                    val bytesPerSec = copied / elapsedSec
                    if (bytesPerSec > 0) ((totalBytes - copied) / bytesPerSec).toInt().coerceAtLeast(0) else 0
                } else 0
                onProgress(pct, etaSec)
            }
        }
    }
    output.flush()
}

/**
 * Map raw FFmpeg failure output to a short, user-facing message. Falls back to a
 * generic retry hint. The full log is still logged separately for debugging.
 */
fun ffmpegFailureMessage(logs: String): String {
    val l = logs.lowercase()
    return when {
        "activation" in l || "invalid data found" in l ->
            "Decryption failed — the activation bytes may be wrong. Try re-authenticating your account."
        "no space left" in l ->
            "Not enough free storage to convert this audiobook."
        "permission denied" in l ->
            "Storage permission denied while saving the audiobook."
        else ->
            "Audio conversion failed. Please retry."
    }
}

/**
 * Rolling ETA estimator for a byte-counted transfer polled over time.
 *
 * Call [update] each poll with the current bytes and total; [etaSeconds] then
 * holds the estimated remaining seconds, based on an exponential moving average
 * of throughput (smooths out the jitter of instantaneous per-poll speed).
 * ETA is held (not zeroed) across stalls so it doesn't flicker.
 */
class SpeedEta {
    private var lastBytes = -1L
    private var lastMs = System.currentTimeMillis()
    private var avgBytesPerSec = 0.0

    var etaSeconds: Long = 0L
        private set

    fun update(bytesDownloaded: Long, totalBytes: Long) {
        val nowMs = System.currentTimeMillis()
        val dtSec = (nowMs - lastMs) / 1000.0
        // Skip the first sample and any non-monotonic reading (resume/restart).
        if (lastBytes in 0..bytesDownloaded && dtSec > 0.0) {
            val instBytesPerSec = (bytesDownloaded - lastBytes) / dtSec
            if (instBytesPerSec > 0.0) {
                avgBytesPerSec = if (avgBytesPerSec <= 0.0) instBytesPerSec
                                 else 0.7 * avgBytesPerSec + 0.3 * instBytesPerSec
            }
        }
        lastBytes = bytesDownloaded
        lastMs = nowMs
        if (avgBytesPerSec > 0.0 && totalBytes > bytesDownloaded) {
            etaSeconds = ((totalBytes - bytesDownloaded) / avgBytesPerSec).toLong().coerceAtLeast(0L)
        }
    }
}
