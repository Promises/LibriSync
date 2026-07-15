package expo.modules.rustbridge

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream

/**
 * Unit tests for the pure-JVM download-progress helpers.
 * Run: ./gradlew :expo-rust-bridge:testDebugUnitTest
 */
class AudioProgressTest {

    @Test
    fun copyStreamWithProgress_copiesAllBytes() {
        val data = ByteArray(1_000_000) { (it % 256).toByte() }
        val out = ByteArrayOutputStream()
        copyStreamWithProgress(ByteArrayInputStream(data), out, data.size.toLong()) { _, _ -> }
        assertEquals(data.size, out.size())
        assertTrue(data.contentEquals(out.toByteArray()))
    }

    @Test
    fun copyStreamWithProgress_reachesHundredPercent() {
        val data = ByteArray(700_000) { 1 }
        var lastPct = -1
        copyStreamWithProgress(ByteArrayInputStream(data), ByteArrayOutputStream(), data.size.toLong()) { pct, _ ->
            lastPct = pct
        }
        assertEquals(100, lastPct)
    }

    @Test
    fun copyStreamWithProgress_monotonicPercent() {
        val data = ByteArray(2_000_000) { 0 }
        var prev = -1
        copyStreamWithProgress(ByteArrayInputStream(data), ByteArrayOutputStream(), data.size.toLong()) { pct, _ ->
            assertTrue("percent must not go backwards", pct >= prev)
            prev = pct
        }
    }

    @Test
    fun copyStreamWithProgress_unknownTotal_noProgressButStillCopies() {
        val data = ByteArray(1000) { 7 }
        val out = ByteArrayOutputStream()
        var called = false
        copyStreamWithProgress(ByteArrayInputStream(data), out, 0L) { _, _ -> called = true }
        assertEquals(1000, out.size())
        assertTrue("no progress callbacks when total is unknown", !called)
    }

    @Test
    fun speedEta_seedsAndStaysNonNegative() {
        val se = SpeedEta()
        assertEquals(0L, se.etaSeconds)
        se.update(0L, 1_000_000L)      // first sample: baseline only
        se.update(200_000L, 1_000_000L) // some progress
        assertTrue("eta never negative", se.etaSeconds >= 0L)
    }

    @Test
    fun speedEta_ignoresNonMonotonicBytes() {
        val se = SpeedEta()
        se.update(500_000L, 1_000_000L)
        se.update(100_000L, 1_000_000L) // went backwards (restart) — must not crash / go negative
        assertTrue(se.etaSeconds >= 0L)
    }

    @Test
    fun ffmpegFailureMessage_mapsActivationBytes() {
        val msg = ffmpegFailureMessage("… Invalid data found when processing input …")
        assertTrue(msg.contains("Decryption failed"))
    }

    @Test
    fun ffmpegFailureMessage_mapsNoSpace() {
        assertTrue(ffmpegFailureMessage("write error: No space left on device").contains("storage"))
    }

    @Test
    fun ffmpegFailureMessage_fallback() {
        assertEquals("Audio conversion failed. Please retry.", ffmpegFailureMessage("some unrelated ffmpeg noise"))
    }
}
