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

data class DownloadPlan(
    val parts: List<DownloadPart>,
    val embedMetadata: Boolean = false,
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
                    "zip" -> parts.add(DownloadPart.ZipPart(p.getString("url"), headers))
                    else -> {}
                }
            }
            return DownloadPlan(parts, json.optBoolean("embed_metadata", false))
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
