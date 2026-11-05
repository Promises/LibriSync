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
 *
 * Also carries the book's title and the notification's speed text, so the Downloads
 * screen can list every book the service is working on — including ones queued in
 * sequential mode, which have no DB task yet — without a lookup per row.
 */
object StageProgressStore {
    data class Entry(
        val stage: String,
        val percentage: Int,
        val etaSeconds: Long,
        val title: String?,
        val speed: String?,
    )

    private val entries = ConcurrentHashMap<String, Entry>()

    /** A null [title] keeps the one already known for [asin]. */
    fun update(
        asin: String,
        stage: String,
        percentage: Int,
        etaSeconds: Long,
        title: String? = null,
        speed: String? = null,
    ) {
        entries[asin] = Entry(
            stage,
            percentage.coerceIn(0, 100),
            etaSeconds.coerceAtLeast(0L),
            title ?: entries[asin]?.title,
            speed,
        )
    }

    fun clear(asin: String) {
        entries.remove(asin)
    }

    /**
     * JSON: { "<asin>": { "stage": String, "percentage": Int, "eta_seconds": Long,
     *                     "title"?: String, "speed"?: String }, ... }
     */
    fun snapshotJson(): String {
        val root = JSONObject()
        for ((asin, e) in entries) {
            root.put(asin, JSONObject().apply {
                put("stage", e.stage)
                put("percentage", e.percentage)
                put("eta_seconds", e.etaSeconds)
                e.title?.let { put("title", it) }
                e.speed?.let { put("speed", it) }
            })
        }
        return root.toString()
    }
}
