package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.model.ManualConnectUiState
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportException
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class ManualListenerTransportControllerTest {
    @Test
    fun everyPostConnectionExceptionMapsToDisconnectedNotFailed() {
        val exceptions = listOf(
            FfiListenerTransportException.InvalidEndpoint("invalid"),
            FfiListenerTransportException.UnsupportedProtocolVersion("version"),
            FfiListenerTransportException.Unauthorized("unauthorized"),
            FfiListenerTransportException.Timeout("timeout"),
            FfiListenerTransportException.ConnectionFailed("connection failed"),
            FfiListenerTransportException.Io("io"),
            FfiListenerTransportException.Closed("transport event channel is closed"),
        )

        for (exception in exceptions) {
            val state = mapPostConnectionFailure(exception)
            assertThat(state).isEqualTo(ManualConnectUiState.Disconnected(exception.message))
        }
    }

    @Test
    fun closedExceptionIsNotRenderedAsAConnectionFailure() {
        val state = mapPostConnectionFailure(
            FfiListenerTransportException.Closed("transport event channel is closed"),
        )

        assertThat(state).isNotInstanceOf(ManualConnectUiState.Failed::class.java)
    }
}
