package expo.modules.rustbridge

import org.json.JSONObject

/**
 * Kotlin mirror of the Rust `providers::DownloadPlan` (see native providers module).
 *
 * A provider produces a typed plan; the download engine ([DownloadOrchestrator.enqueuePlan])
 * executes it without knowing which provider it came from. Each part carries how to turn it
 * into a finished file:
 *  - [PlainPart]  — copy as-is (LibriVox mp3, Libro.fm packaged m4b)
 *  - [AaxcPart]   — FFmpeg-decrypt with key/iv (Audible)
 *  - [ZipPart]    — a ZIP of audio files, extracted into the book folder
 */
sealed class DownloadPart {
    abstract val url: String
    abstract val headers: Map<String, String>

    data class PlainPart(
        override val url: String,
        override val headers: Map<String, String>,
        val filename: String,
    ) : DownloadPart()

    /**
     * Legacy AAX: decrypted with per-account activation bytes (FFmpeg
     * `-activation_bytes`) rather than a per-book key/iv pair. Produced by the CDE
     * fallback when Audible refuses a download licence.
     */
    data class AaxPart(
        override val url: String,
        override val headers: Map<String, String>,
        val activationBytes: String,
        val filename: String,
    ) : DownloadPart()

    data class AaxcPart(
        override val url: String,
        override val headers: Map<String, String>,
        val key: String,
        val iv: String,
        val filename: String,
    ) : DownloadPart()

    data class ZipPart(
        override val url: String,
        override val headers: Map<String, String>,
    ) : DownloadPart()
}

/**
 * One chapter marker supplied by the provider (Audible license chapter_info,
 * Libro.fm manifest tracks). Times are milliseconds from the start of the book.
 * Used when the user asked for per-chapter output.
 */
data class PlanChapter(
    val title: String,
    val startMs: Long,
    val endMs: Long,
)

data class DownloadPlan(
    val parts: List<DownloadPart>,
    val embedMetadata: Boolean = false,
    val chapters: List<PlanChapter> = emptyList(),
) {
    companion object {
        /** Parse a plan from the JSON emitted by `nativeProviderGetDownloadPlan`. */
        fun fromJson(json: JSONObject): DownloadPlan {
            val partsJson = json.optJSONArray("parts") ?: org.json.JSONArray()
            val parts = ArrayList<DownloadPart>(partsJson.length())
            for (i in 0 until partsJson.length()) {
                val p = partsJson.getJSONObject(i)
                val headers = headersOf(p.optJSONObject("headers"))
                when (p.optString("kind")) {
                    "plain" -> parts.add(
                        DownloadPart.PlainPart(p.getString("url"), headers, p.getString("filename"))
                    )
                    "aaxc" -> parts.add(
                        DownloadPart.AaxcPart(
                            p.getString("url"), headers,
                            p.getString("key"), p.getString("iv"), p.getString("filename")
                        )
                    )
                    "aax" -> parts.add(
                        DownloadPart.AaxPart(
                            p.getString("url"), headers,
                            p.getString("activation_bytes"), p.getString("filename")
                        )
                    )
                    "zip" -> parts.add(DownloadPart.ZipPart(p.getString("url"), headers))
                    else -> {}
                }
            }
            return DownloadPlan(parts, json.optBoolean("embed_metadata", false), chaptersOf(json))
        }

        /** Parse `chapters[]`; a provider that supplies none yields an empty list. */
        private fun chaptersOf(json: JSONObject): List<PlanChapter> {
            val arr = json.optJSONArray("chapters") ?: return emptyList()
            val out = ArrayList<PlanChapter>(arr.length())
            for (i in 0 until arr.length()) {
                val c = arr.optJSONObject(i) ?: continue
                val start = c.optLong("start_ms", -1L)
                val end = c.optLong("end_ms", -1L)
                // Drop malformed/zero-length markers rather than emitting empty files.
                if (start < 0 || end <= start) continue
                out.add(PlanChapter(c.optString("title"), start, end))
            }
            return out
        }

        private fun headersOf(obj: JSONObject?): Map<String, String> {
            if (obj == null) return emptyMap()
            val out = HashMap<String, String>()
            val keys = obj.keys()
            while (keys.hasNext()) {
                val k = keys.next()
                out[k] = obj.getString(k)
            }
            return out
        }
    }
}
