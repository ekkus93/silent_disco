package com.ekkus.silentdisco.core.audio

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class CallbackOutputLifecycleTest {
    private fun report(start: String, end: String) = CallbackAudioCaptureReport(
        file = File("capture.wav"),
        wavFrames = 480,
        capturedFrames = 480u,
        lostFrames = 0u,
        callbackCount = 3u,
        outputLifecycleAtStart = start,
        outputLifecycleAtEnd = end,
    )

    @Test
    fun retainedNativeLifecycleCountsDetectUnexpectedReopen() {
        val begin = "opens=1 rebinds=0 closes=0 sampleRate=48000 channels=2"
        val sameStream = "opens=1 rebinds=2 closes=0 sampleRate=48000 channels=2"
        assertEquals(OutputLifecycle(1, 0, 0), parseOutputLifecycle(begin))
        assertTrue(report(begin, sameStream).noUnexpectedOutputReopen == true)
        val restarted = "opens=2 rebinds=2 closes=1 sampleRate=48000 channels=2"
        assertFalse(report(begin, restarted).noUnexpectedOutputReopen!!)
    }

    @Test
    fun missingNativeSummaryDoesNotSilentlyPassLifecycleEvidence() {
        assertNull(parseOutputLifecycle("Native bridge unavailable"))
        assertNull(report("Native bridge unavailable", "opens=2 rebinds=0 closes=1")
            .noUnexpectedOutputReopen)
    }

    @Test
    fun incompletePcmCaptureFailsEvenWithoutNativeStreamReopen() {
        val report = report("opens=1 rebinds=0 closes=0", "opens=1 rebinds=0 closes=0")
        assertTrue(report.complete)
        assertFalse(report.copy(lostFrames = 1u).complete)
        assertFalse(report.copy(capturedFrames = 479u).complete)
    }
}
