package com.ekkus.silentdisco.app

import android.net.Uri
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class SupportReportTest {
    @Test
    fun reportOmitsInviteCodeAndInternalIdentifiers() {
        val state = AppUiState(
            hostForm = HostFormState(
                sessionName = "Test Session",
                approvalMode = ApprovalMode.INVITE_CODE,
                inviteCode = "4827",
                selectedAudio = SelectedAudioFile(
                    uri = Uri.parse("content://private/audio/42"),
                    displayName = "audio.flac",
                    mimeType = "audio/flac",
                    sizeBytes = 1024L,
                ),
            ),
            hostDiagnostics = AppUiState().hostDiagnostics.copy(
                sessionId = "secret-session-id",
                metricsSummary = "session=secret-session-id code=4827",
            ),
            listenerDiagnostics = AppUiState().listenerDiagnostics.copy(
                sessionId = "listener-session-id",
                metricsSummary = "source=content://private/audio/42",
            ),
            lastError = "Join rejected for secret-session-id using code 4827",
        )

        val report = state.buildSupportReport(
            appVersion = "1.2.3",
            generatedAt = "2026-07-26T12:00:00Z",
        )

        assertThat(report).contains("Silent Disco support report")
        assertThat(report).contains("App version: 1.2.3")
        assertThat(report).contains("[redacted]")
        assertThat(report).doesNotContain("4827")
        assertThat(report).doesNotContain("secret-session-id")
        assertThat(report).doesNotContain("listener-session-id")
        assertThat(report).doesNotContain("content://private/audio/42")
    }
}
