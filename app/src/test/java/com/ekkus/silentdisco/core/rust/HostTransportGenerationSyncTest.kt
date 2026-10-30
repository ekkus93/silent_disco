package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiHostTransportDelivery
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportHandle
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.test.runTest
import org.junit.Test
import org.mockito.Mockito

/**
 * The real Android controller must retain the native handle associated with
 * the received SyncRequest. A queued old-session response cannot be sent on
 * a replacement host even when both sessions use the same listener ID.
 */
class HostTransportGenerationSyncTest {
    @Test
    fun oldGenerationSyncIsRefusedAfterRebindAndCurrentGenerationCanReply() = runTest {
        val old = Mockito.mock(FfiHostTransportHandle::class.java)
        val replacement = Mockito.mock(FfiHostTransportHandle::class.java)
        for (handle in listOf(old, replacement)) {
            // The production poller runs on IO. Keep the mocked native poll
            // bounded while the controller exercises actual bind/rebind/close.
            Mockito.doAnswer {
                Thread.sleep(10)
                null
            }.`when`(handle).pollEvent(500uL)
        }
        val opened = ArrayDeque(listOf(old, replacement))
        var sendClock = 100uL
        val controller = HostTransportController(
            callDispatcher = Dispatchers.Unconfined,
            bindHandle = { _, _, _, _, _ -> opened.removeFirst() },
            hostElapsedMs = { sendClock },
        )
        try {
            controller.bind(backgroundScope, "127.0.0.1", 1, 2, 3, "old-session")
            controller.bind(backgroundScope, "127.0.0.1", 1, 2, 3, "new-session")
            val failure = runCatching {
                controller.sendSyncResponseForGeneration(
                    old, 42uL, 100uL, 110uL,
                )
            }.exceptionOrNull()
            assertThat(failure).isInstanceOf(IllegalStateException::class.java)
            assertThat(failure?.message).contains("disconnected before synchronization response delivery")
            Mockito.verify(old, Mockito.never()).sendSyncResponse(42uL, 100uL, 110uL, 120uL)
            Mockito.verify(replacement, Mockito.never()).sendSyncResponse(42uL, 100uL, 110uL, 120uL)

            val delivered = FfiHostTransportDelivery(
                intendedPeers = 1u,
                successfulPeers = 1u,
                failedPeers = 0u,
                bytesSent = 24uL,
            )
            // A reply queued earlier uses the actual send clock sampled
            // after rebind/dispatcher delay, never the old event clock.
            sendClock = 777uL
            Mockito.`when`(replacement.sendSyncResponse(43uL, 200uL, 210uL, 777uL))
                .thenReturn(delivered)
            assertThat(controller.sendSyncResponseForGeneration(
                replacement, 43uL, 200uL, 210uL,
            )).isEqualTo(delivered)
            Mockito.verify(replacement).sendSyncResponse(43uL, 200uL, 210uL, 777uL)
        } finally {
            controller.close()
        }
    }
}
