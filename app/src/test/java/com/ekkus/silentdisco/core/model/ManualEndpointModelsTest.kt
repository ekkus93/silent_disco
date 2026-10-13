package com.ekkus.silentdisco.core.model

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class ManualEndpointModelsTest {
    @Test
    fun sessionDisplayUsesCompactLabelForOpaqueHostIdentity() {
        val session = SessionInfo(
            id = "session",
            name = "Friday set",
            hostDeviceName = "0123456789abcdef".repeat(4),
            approvalMode = ApprovalMode.MANUAL,
            inviteCodeRequired = false,
        )

        assertThat(session.displayHostName).isEqualTo("Host 01234567")
        assertThat(session.hostDeviceName).isEqualTo("0123456789abcdef".repeat(4))
    }

    @Test
    fun sessionDisplayPreservesFriendlyHostName() {
        val session = SessionInfo(
            id = "session",
            name = "Friday set",
            hostDeviceName = "Studio Mac",
            approvalMode = ApprovalMode.MANUAL,
            inviteCodeRequired = false,
        )

        assertThat(session.displayHostName).isEqualTo("Studio Mac")
    }

    @Test
    fun cannotConnectUntilValidatedWithoutError() {
        assertThat(ManualEndpointFormState().canConnect()).isFalse()
        assertThat(
            ManualEndpointFormState(hostAddress = "192.168.1.50", sessionId = "s").canConnect(),
        ).isTrue()
        assertThat(
            ManualEndpointFormState(
                hostAddress = "192.168.1.50",
                sessionId = "s",
                validationError = "bad",
            ).canConnect(),
        ).isFalse()
    }

    @Test
    fun inviteCodeRequiredBlocksConnectUntilEntered() {
        val form = ManualEndpointFormState(
            hostAddress = "192.168.1.50",
            sessionId = "s",
            inviteCodeRequired = true,
        )
        assertThat(form.canConnect()).isFalse()
        assertThat(form.copy(inviteCode = "1234").canConnect()).isTrue()
    }

    @Test
    fun connectingAndAwaitingApprovalAreInProgress() {
        assertThat(ManualConnectUiState.Idle.isInProgress()).isFalse()
        assertThat(ManualConnectUiState.Connecting.isInProgress()).isTrue()
        assertThat(
            ManualConnectUiState.AwaitingApproval("host", "session").isInProgress(),
        ).isTrue()
        assertThat(ManualConnectUiState.Approved(false).isInProgress()).isFalse()
        assertThat(ManualConnectUiState.Rejected("no").isInProgress()).isFalse()
        assertThat(
            ManualConnectUiState.Streaming(true, PlaybackState.PLAYING).isInProgress(),
        ).isFalse()
    }
}
