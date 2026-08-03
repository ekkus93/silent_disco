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
        // Should either return actual native summary or a failure message containing "native"
        val hasNativeWord = summary.contains("native", ignoreCase = true) || summary.contains("Native", ignoreCase = true)
        val hasContent = summary.contains("AAudio") || summary.contains("OpenSLES")
        assertThat(hasNativeWord || hasContent).isTrue()
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
        // Should either return actual native status or a message indicating unavailability
        val hasUnavailableWord = summary.contains("Unavailable", ignoreCase = true) || summary.contains("unavailable", ignoreCase = true)
        val hasContent = summary.contains("frames") || summary.contains("underrun")
        assertThat(hasUnavailableWord || hasContent).isTrue()
    }

    @Test
    fun diagnosticStrings_clarifyNativeVsManaged() {
        val backend = OboeBridge.backendSummary()
        val status = OboeBridge.statusSummary()

        // Strings should clarify what's available
        assertThat(backend).isNotEqualTo("")
        assertThat(status).isNotEqualTo("")

        // If native is unavailable, message should clearly indicate it
        if (!OboeBridge.isAvailable) {
            assertThat(backend.lowercase()).containsMatch("native|unavailable|not loaded")
        }
    }
}
