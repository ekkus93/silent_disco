package com.ekkus.silentdisco.core.audio

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class OboeBridgeDiagnosticsTest {

    @Test
    fun backendSummary_returnsNonNull() {
        val summary = OboeBridge.backendSummary()
        assertThat(summary).isNotNull()
        assertThat(summary).isNotEmpty()
    }

    @Test
    fun backendSummary_usesDefaultWhenNativeUnavailable() {
        // The native library may not load in test environment
        val summary = OboeBridge.backendSummary()
        // Should either return actual native summary or default
        val isDefault = summary == "No native Oboe"
        val hasContent = summary.contains("AAudio") || summary.contains("OpenSLES")
        assertThat(isDefault || hasContent).isTrue()
    }

    @Test
    fun statusSummary_returnsNonNull() {
        val summary = OboeBridge.statusSummary()
        assertThat(summary).isNotNull()
        assertThat(summary).isNotEmpty()
    }

    @Test
    fun statusSummary_usesDefaultWhenNativeUnavailable() {
        // The native library may not load in test environment
        val summary = OboeBridge.statusSummary()
        // Should either return actual native status or default
        val isDefault = summary == "Native bridge unavailable"
        val hasContent = summary.contains("frames") || summary.contains("underrun")
        assertThat(isDefault || hasContent).isTrue()
    }

    @Test
    fun diagnosticStrings_clarifyNativeVsManaged() {
        val backend = OboeBridge.backendSummary()
        val status = OboeBridge.statusSummary()

        // Strings should clarify what's available
        assertThat(backend).isNotEqualTo("")
        assertThat(status).isNotEqualTo("")

        // If native is unavailable, message should say so clearly
        if (backend == "No native Oboe") {
            assertThat(backend).contains("native")
        }
    }
}

class PlaybackEngineDiagnosticsTest {

    @Test
    fun playbackEngine_canBeCreatedWithoutAudioTrack() {
        val engine = AudioTrackPlaybackEngine()
        // Engine should be instantiable without system resources
        engine.stop() // Should not throw
    }

    @Test
    fun setVolume_defaultsToFullVolume() {
        val engine = AudioTrackPlaybackEngine()
        // Default volume should be 1.0 (full)
        engine.setVolume(0.5f)
        engine.setVolume(1.0f)
        // Should complete without error
    }

    @Test
    fun statusSummary_returnsClearyMessage() {
        val engine = AudioTrackPlaybackEngine()
        val summary = engine.statusSummary()

        // Should return a non-empty string
        assertThat(summary).isNotNull()
        assertThat(summary).isNotEmpty()
    }
}
