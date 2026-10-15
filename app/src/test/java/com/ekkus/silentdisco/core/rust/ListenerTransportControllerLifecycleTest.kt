package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportException
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class ListenerTransportControllerLifecycleTest {
    @Test
    fun closedPollIsTreatedAsNormalHostShutdown() {
        assertThat(
            polledConnectionClosedMessage(
                FfiListenerTransportException.Closed("transport event channel is closed"),
            ),
        ).isNull()
    }

    @Test
    fun unexpectedPollFailureRetainsItsDiagnostic() {
        assertThat(
            polledConnectionClosedMessage(FfiListenerTransportException.Io("socket failed")),
        ).isEqualTo("socket failed")
    }

    @Test
    fun pollResultIsIgnoredAfterItsLoopIsCancelledOrHandleReplaced() {
        assertThat(shouldForwardPolledListenerEvent(loopActive = false, handleIsCurrent = true))
            .isFalse()
        assertThat(shouldForwardPolledListenerEvent(loopActive = true, handleIsCurrent = false))
            .isFalse()
        assertThat(shouldForwardPolledListenerEvent(loopActive = true, handleIsCurrent = true))
            .isTrue()
    }

    @Test
    fun finalLeaveWriteFailureAfterPeerCloseDoesNotBecomeCleanupFailure() {
        assertThat(
            isPeerClosedDisconnectWriteFailure(
                FfiListenerTransportException.Io(
                    "control transport Delivery: failed to write control frame: " +
                        "Broken pipe (os error 32)",
                ),
            ),
        ).isTrue()
    }

    @Test
    fun unrelatedDisconnectWriteFailureRemainsVisible() {
        assertThat(
            isPeerClosedDisconnectWriteFailure(
                FfiListenerTransportException.Io(
                    "control transport Delivery: failed to write control frame: " +
                        "Permission denied (os error 13)",
                ),
            ),
        ).isFalse()
        assertThat(
            isPeerClosedDisconnectWriteFailure(FfiListenerTransportException.Closed("already closed")),
        ).isFalse()
    }
}
