package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class HostTransportEventGateTest {
    @Test
    fun activeCurrentHostCanForwardJoinAndDisconnectEvents() {
        assertThat(shouldForwardPolledHostEvent(loopActive = true, handleIsCurrent = true))
            .isTrue()
    }

    @Test
    fun cancelledPollCannotReportTeardownAsNewHostFailure() {
        assertThat(shouldForwardPolledHostEvent(loopActive = false, handleIsCurrent = true))
            .isFalse()
    }

    @Test
    fun oldHostPollCannotLeakEventsIntoReboundSession() {
        assertThat(shouldForwardPolledHostEvent(loopActive = true, handleIsCurrent = false))
            .isFalse()
        assertThat(shouldForwardPolledHostEvent(loopActive = false, handleIsCurrent = false))
            .isFalse()
    }
}
