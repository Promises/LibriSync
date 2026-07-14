package expo.modules.rustbridge

import java.util.concurrent.ConcurrentHashMap
import org.json.JSONObject

/**
 * In-memory, per-ASIN progress for the post-download pipeline stages
 * (decrypting / validating / copying) and the live download ETA.
 *
 * The download-task DB row only carries status + byte counts, so stage
 * percentage and ETA — which the notification already computes — would
 * otherwise be invisible to the JS UI. The foreground download pipeline
 * writes here on every progress tick; [LibraryScreen] polls [snapshotJson]
 * alongside its existing 2s task poll. Cleared when a book finishes or fails.
 */
object StageProgressStore {
    data class Entry(val stage: String, val percentage: Int, val etaSeconds: Long)

    private val entries = ConcurrentHashMap<String, Entry>()

    fun update(asin: String, stage: String, percentage: Int, etaSeconds: Long) {
        entries[asin] = Entry(stage, percentage.coerceIn(0, 100), etaSeconds.coerceAtLeast(0L))
    }

    fun clear(asin: String) {
        entries.remove(asin)
    }

    /** JSON: { "<asin>": { "stage": String, "percentage": Int, "eta_seconds": Long }, ... } */
    fun snapshotJson(): String {
        val root = JSONObject()
        for ((asin, e) in entries) {
            root.put(asin, JSONObject().apply {
                put("stage", e.stage)
                put("percentage", e.percentage)
                put("eta_seconds", e.etaSeconds)
            })
        }
        return root.toString()
    }
}
