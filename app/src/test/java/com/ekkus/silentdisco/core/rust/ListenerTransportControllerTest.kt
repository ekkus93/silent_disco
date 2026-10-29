package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.test.runTest
import org.junit.Test

class ListenerTransportControllerTest {
    @Test
    fun joinBeforeConnectFailsInsteadOfSilentlyWaitingForApproval() = runTest {
        val controller = ListenerTransportController()
        try {
            val failure = runCatching {
                controller.sendJoinRequest("Android listener", null)
            }.exceptionOrNull()
            assertThat(failure).isInstanceOf(IllegalStateException::class.java)
            assertThat(failure?.message).contains("disconnected before join request delivery")
        } finally {
            controller.close()
        }
    }

    @Test
    fun joinAfterDisconnectFailsAndDoesNotResurrectRetiredSession() = runTest {
        val controller = ListenerTransportController()
        try {
            controller.disconnect("previous session ended")
            val failure = runCatching {
                controller.sendJoinRequest("Android listener", "123456")
            }.exceptionOrNull()
            assertThat(failure).isInstanceOf(IllegalStateException::class.java)
            assertThat(failure?.message).contains("disconnected before join request delivery")
        } finally {
            controller.close()
        }
    }
}
