package com.ekkus.silentdisco.feature.host

import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.HostFormState
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class InviteSessionPresentationTest {
    @Test
    fun manualInvitationExplainsHostApproval() {
        val state = AppUiState(
            hostForm = HostFormState(
                sessionName = "Kitchen Disco",
                approvalMode = ApprovalMode.MANUAL,
            ),
        )

        val instructions = inviteInstructions(state)

        assertThat(instructions).contains("Kitchen Disco")
        assertThat(instructions).contains("host will approve")
    }

    @Test
    fun codeInvitationIncludesCodeButNeverSessionId() {
        val baseline = AppUiState(
            hostForm = HostFormState(
                sessionName = "Kitchen Disco",
                approvalMode = ApprovalMode.INVITE_CODE,
                inviteCode = "4827",
            ),
        )
        val state = baseline.copy(
            hostDiagnostics = baseline.hostDiagnostics.copy(sessionId = "internal-session-id"),
        )

        val instructions = inviteInstructions(state)

        assertThat(instructions).contains("4827")
        assertThat(instructions).doesNotContain("internal-session-id")
    }

    @Test
    fun approvedDeviceInvitationExplainsRestriction() {
        val state = AppUiState(
            hostForm = HostFormState(
                approvalMode = ApprovalMode.TRUSTED_DEVICES_PLACEHOLDER,
            ),
        )

        assertThat(inviteInstructions(state)).contains("always allowed")
    }
}
