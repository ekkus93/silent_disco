package com.ekkus.silentdisco.core.audio

import java.io.File
import java.nio.file.Files
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.assertThrows
import org.junit.Test

class CallbackPcmWavWriterTest {
    private fun temporaryFile(): File {
        val directory = Files.createTempDirectory("callback-pcm-test").toFile()
        directory.deleteOnExit()
        return File(directory, "callback.wav").also { it.deleteOnExit() }
    }

    @Test
    fun callbackPcmIsWrittenAsLittleEndianStereoWavWithExactLength() {
        val file = temporaryFile()
        val writer = CallbackPcmWavWriter(file, sampleRate = 48_000)
        writer.append(listOf(0f, 1f, -1f, 0.5f))
        assertEquals(2L, writer.finish())
        assertEquals(2L, writer.finish())
        val bytes = file.readBytes()
        assertArrayEquals("RIFF".toByteArray(), bytes.copyOfRange(0, 4))
        assertArrayEquals("WAVE".toByteArray(), bytes.copyOfRange(8, 12))
        assertEquals(52, bytes.size)
        assertArrayEquals(
            byteArrayOf(0, 0, -1, 127, 0, -128, 0, 64),
            bytes.copyOfRange(44, 52),
        )
    }

    @Test
    fun partialFramesAndExcessiveCaptureFailExplicitly() {
        val file = temporaryFile()
        val writer = CallbackPcmWavWriter(file, maxFrames = 1)
        assertThrows(IllegalArgumentException::class.java) { writer.append(listOf(1f)) }
        writer.append(listOf(0.1f, 0.2f))
        assertThrows(IllegalStateException::class.java) { writer.append(listOf(0.3f, 0.4f)) }
        writer.finish()
        assertThrows(IllegalStateException::class.java) { writer.append(listOf(0.5f, 0.6f)) }
    }

    @Test
    fun invalidWavGeometryDoesNotOpenOrCreateACaptureFile() {
        val file = temporaryFile()
        assertFalse(file.exists())
        assertThrows(IllegalArgumentException::class.java) {
            CallbackPcmWavWriter(file, sampleRate = 0)
        }
        assertThrows(IllegalArgumentException::class.java) {
            CallbackPcmWavWriter(file, sampleRate = Int.MAX_VALUE)
        }
        assertThrows(IllegalArgumentException::class.java) {
            CallbackPcmWavWriter(file, maxFrames = 0)
        }
        assertThrows(IllegalArgumentException::class.java) {
            CallbackPcmWavWriter(file, maxFrames = Long.MAX_VALUE)
        }
        assertFalse(file.exists())
    }

    @Test
    fun nonFiniteCallbackBatchDoesNotCorruptPreviouslyCapturedPcm() {
        val file = temporaryFile()
        val writer = CallbackPcmWavWriter(file)
        writer.append(listOf(0.125f, -0.125f))
        assertThrows(IllegalArgumentException::class.java) {
            writer.append(listOf(0.5f, Float.NaN))
        }
        assertThrows(IllegalArgumentException::class.java) {
            writer.append(listOf(Float.POSITIVE_INFINITY, 0.5f))
        }
        writer.append(listOf(0.25f, -0.25f))
        assertEquals(2L, writer.finish())
        val bytes = file.readBytes()
        assertEquals(52, bytes.size)
        assertArrayEquals(
            byteArrayOf(0, 16, 0, -16, 0, 32, 0, -32),
            bytes.copyOfRange(44, 52),
        )
    }

    @Test
    fun captureReportRequiresLiveOutputThroughoutCapture() {
        fun report(start: String, end: String): CallbackAudioCaptureReport =
            CallbackAudioCaptureReport(
                file = temporaryFile(),
                wavFrames = 100,
                capturedFrames = 100uL,
                lostFrames = 0uL,
                callbackCount = 2uL,
                tapFullyDrained = true,
                outputLifecycleAtStart = start,
                outputLifecycleAtEnd = end,
            )
        val live = "opens=1 rebinds=0 closes=0"
        val closed = "opens=1 rebinds=0 closes=1"
        assertTrue(report(live, live).complete)
        assertEquals(true, report(live, live).noUnexpectedOutputReopen)
        // Rebinding an existing stream is permitted, but actually closing
        // the stream or opening its replacement is not gapless.
        assertEquals(true, report(live, "opens=1 rebinds=1 closes=0").noUnexpectedOutputReopen)
        assertEquals(
            true,
            report("opens=2 rebinds=1 closes=1", "opens=2 rebinds=2 closes=1")
                .noUnexpectedOutputReopen,
        )
        // An unchanged but impossible output state cannot certify continuity.
        assertFalse(report("opens=2 rebinds=0 closes=0", "opens=2 rebinds=0 closes=0")
            .noUnexpectedOutputReopen!!)
        assertFalse(report("opens=1 rebinds=2 closes=0", "opens=1 rebinds=1 closes=0")
            .noUnexpectedOutputReopen!!)
        assertNull(report(live, "opens=1 opens=2 rebinds=0 closes=0").noUnexpectedOutputReopen)
        assertFalse(report(closed, closed).noUnexpectedOutputReopen!!)
        assertFalse(report(live, closed).noUnexpectedOutputReopen!!)
        assertFalse(report(live, "opens=2 rebinds=0 closes=1").noUnexpectedOutputReopen!!)
        assertNull(report(live, "output unavailable").noUnexpectedOutputReopen)
        assertNull(report(live, "opens=-1 rebinds=0 closes=0").noUnexpectedOutputReopen)
    }

    @Test
    fun callbackCaptureReportRejectsEmptyTruncatedOrLostRecording() {
        fun evidence(wavFrames: Long, capturedFrames: ULong, lostFrames: ULong, callbacks: ULong) =
            CallbackAudioCaptureReport(
                file = temporaryFile(),
                wavFrames = wavFrames,
                capturedFrames = capturedFrames,
                lostFrames = lostFrames,
                callbackCount = callbacks,
                tapFullyDrained = true,
                outputLifecycleAtStart = "opens=1 rebinds=0 closes=0",
                outputLifecycleAtEnd = "opens=1 rebinds=0 closes=0",
                outputCountersAtStart = OutputCallbackCounters(2, 12),
                outputCountersAtEnd = OutputCallbackCounters(3, 14),
            )
        assertTrue(evidence(10, 10uL, 0uL, 1uL).complete)
        assertFalse(evidence(10, 10uL, 0uL, 1uL).copy(tapFullyDrained = false).complete)
        assertFalse(evidence(0, 0uL, 0uL, 1uL).complete)
        assertFalse(evidence(10, 10uL, 1uL, 1uL).complete)
        assertFalse(evidence(9, 10uL, 0uL, 1uL).complete)
        assertFalse(evidence(10, 10uL, 0uL, 0uL).complete)
        assertEquals(1L, evidence(10, 10uL, 0uL, 1uL).newUnderrunCallbacks)
        assertEquals(2L, evidence(10, 10uL, 0uL, 1uL).newSilenceFilledFrames)
        assertNull(outputCounterDelta(5, 4))
    }

}
