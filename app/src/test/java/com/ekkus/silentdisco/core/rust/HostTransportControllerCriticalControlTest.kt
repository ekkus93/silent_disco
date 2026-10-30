package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.test.runTest
import org.junit.Test

/** Required clock-control sends must fail visibly after Stop or close. */
class HostTransportControllerCriticalControlTest {
    @Test
    fun syncResponseBeforeBindCannotBeMisreportedAsDelivered() = runTest {
        val controller = HostTransportController()
        try {
            val failure = runCatching {
                controller.sendSyncResponse(
                    correlationId = 17uL,
                    t1ListenerSendElapsedMs = 100uL,
                    t2HostReceiveElapsedMs = 110uL,
                    t3HostSendElapsedMs = 120uL,
                )
            }.exceptionOrNull()
            assertThat(failure).isInstanceOf(IllegalStateException::class.java)
            assertThat(failure?.message).contains("disconnected before synchronization response delivery")
        } finally {
            controller.close()
        }
    }

    @Test
    fun syncResponseAfterCloseCannotBeSentToRetiredSession() = runTest {
        val controller = HostTransportController()
        try {
            controller.close()
            val failure = runCatching {
                controller.sendSyncResponse(
                    correlationId = 18uL,
                    t1ListenerSendElapsedMs = 200uL,
                    t2HostReceiveElapsedMs = 210uL,
                    t3HostSendElapsedMs = 220uL,
                )
            }.exceptionOrNull()
            assertThat(failure).isInstanceOf(IllegalStateException::class.java)
            assertThat(failure?.message).contains("disconnected before synchronization response delivery")
        } finally {
            controller.close()
        }
    }
}
