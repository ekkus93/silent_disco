package com.ekkus.silentdisco.core.model

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class ManualEndpointModelsTest {
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
    }
}
