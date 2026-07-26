package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.ApprovalMode
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
            ),
            hostDiagnostics = AppUiState().hostDiagnostics.copy(sessionId = "secret-session-id"),
            listenerDiagnostics = AppUiState().listenerDiagnostics.copy(sessionId = "listener-session-id"),
        )

        val report = state.buildSupportReport(
            appVersion = "1.2.3",
            generatedAt = "2026-07-26T12:00:00Z",
        )

        assertThat(report).contains("Silent Disco support report")
        assertThat(report).contains("App version: 1.2.3")
        assertThat(report).doesNotContain("4827")
        assertThat(report).doesNotContain("secret-session-id")
        assertThat(report).doesNotContain("listener-session-id")
    }
}
