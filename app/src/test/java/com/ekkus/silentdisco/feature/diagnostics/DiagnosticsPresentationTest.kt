package com.ekkus.silentdisco.feature.diagnostics

import com.ekkus.silentdisco.app.AppUiState
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class DiagnosticsPresentationTest {
    @Test
    fun shareSummaryOmitsInternalSessionIdentifiers() {
        val baseline = AppUiState()
        val state = baseline.copy(
            hostDiagnostics = baseline.hostDiagnostics.copy(
                sessionId = "secret-host-session",
                connectedListenerCount = 2,
            ),
            listenerDiagnostics = baseline.listenerDiagnostics.copy(
                sessionId = "secret-listener-session",
            ),
        )

        val summary = diagnosticsShareSummary(state)

        assertThat(summary).contains("Connected listeners: 2")
        assertThat(summary).doesNotContain("secret-host-session")
        assertThat(summary).doesNotContain("secret-listener-session")
    }
}
