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
        // A close without a matching reopen also invalidates the captured interval.
        val stopped = "opens=1 rebinds=2 closes=1 sampleRate=48000 channels=2"
        assertFalse(report(begin, stopped).noUnexpectedOutputReopen!!)
        // Retained diagnostics alone do not prove a stream was open at capture start.
        assertFalse(report("opens=0 rebinds=0 closes=0", begin).noUnexpectedOutputReopen!!)
    }

    @Test
    fun nativeUnderrunAndSilenceDeltasRequireMonotonicAvailableCounters() {
        val before = OutputCallbackCounters(5, 100)
        val clean = report("opens=1 rebinds=0 closes=0", "opens=1 rebinds=0 closes=0")
            .copy(outputCountersAtStart = before, outputCountersAtEnd = before)
        assertEquals(0L, clean.newUnderrunCallbacks)
        assertEquals(0L, clean.newSilenceFilledFrames)
        val gap = clean.copy(outputCountersAtEnd = OutputCallbackCounters(6, 102))
        assertEquals(1L, gap.newUnderrunCallbacks)
        assertEquals(2L, gap.newSilenceFilledFrames)
        assertNull(clean.copy(outputCountersAtEnd = null).newUnderrunCallbacks)
        assertNull(clean.copy(outputCountersAtEnd = OutputCallbackCounters(4, 99)).newSilenceFilledFrames)
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
        assertFalse(report.copy(callbackCount = 0u).complete)
        assertFalse(report.copy(wavFrames = 0, capturedFrames = 0u).complete)
    }
}
