package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiHostTransportDelivery
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportHandle
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.test.runTest
import org.junit.Test
import org.mockito.Mockito

/**
 * Exercises the production controller's bind/rebind/approval gate. A late
 * Rust actor effect must not authorize a newly bound socket with the old
 * request's claimed device ID, even if the ID string is identical.
 */
class HostTransportGenerationAdmissionTest {
    @Test
    fun delayedOldJoinApprovalCannotMigrateToReplacementNativeHost() = runTest {
        val old = Mockito.mock(FfiHostTransportHandle::class.java)
        val replacement = Mockito.mock(FfiHostTransportHandle::class.java)
        for (handle in listOf(old, replacement)) {
            Mockito.doAnswer {
                Thread.sleep(10)
                null
            }.`when`(handle).pollEvent(500uL)
        }
        val opened = ArrayDeque(listOf(old, replacement))
        val controller = HostTransportController(
            callDispatcher = Dispatchers.Unconfined,
            bindHandle = { _, _, _, _, _ -> opened.removeFirst() },
        )
        try {
            controller.bind(backgroundScope, "127.0.0.1", 1, 2, 3, "session-before")
            controller.bind(backgroundScope, "127.0.0.1", 1, 2, 3, "session-after")
            val stale = controller.approveAndAuthorizeListener(
                "reused-device-id", false, 111u, 112u, expectedGeneration = old,
            )
            assertThat(stale).isNull()
            Mockito.verify(old, Mockito.never()).sendJoinApproval("reused-device-id", false)
            Mockito.verify(replacement, Mockito.never()).sendJoinApproval("reused-device-id", false)

            val delivered = FfiHostTransportDelivery(
                intendedPeers = 1u,
                successfulPeers = 1u,
                failedPeers = 0u,
                bytesSent = 32uL,
            )
            Mockito.`when`(replacement.sendJoinApproval("reused-device-id", false))
                .thenReturn(delivered)
            val fresh = controller.approveAndAuthorizeListener(
                "reused-device-id", false, 211u, 212u, expectedGeneration = replacement,
            )
            assertThat(fresh?.authorized).isTrue()
            assertThat(fresh?.delivery).isEqualTo(delivered)
            Mockito.verify(replacement).authorizeListener("reused-device-id", 211u, 212u)
            Mockito.verify(replacement, Mockito.never()).authorizeListener("reused-device-id", 111u, 112u)
        } finally {
            controller.close()
        }
    }

    @Test
    fun failedNativeAuthorizationDisconnectsPeerAndCannotReportSuccess() = runTest {
        val native = Mockito.mock(FfiHostTransportHandle::class.java)
        Mockito.doAnswer { Thread.sleep(10); null }.`when`(native).pollEvent(500uL)
        val controller = HostTransportController(
            callDispatcher = Dispatchers.Unconfined,
            bindHandle = { _, _, _, _, _ -> native },
        )
        try {
            controller.bind(backgroundScope, "127.0.0.1", 1, 2, 3, "one-session")
            Mockito.`when`(native.sendJoinApproval("listener", false)).thenReturn(
                FfiHostTransportDelivery(1u, 1u, 0u, 32uL),
            )
            Mockito.doThrow(IllegalStateException("native UDP route refused"))
                .`when`(native).authorizeListener("listener", 300u, 301u)
            val result = controller.approveAndAuthorizeListener(
                "listener", false, 300u, 301u, expectedGeneration = native,
            )
            assertThat(result?.authorized).isFalse()
            assertThat(result?.failure).contains("native UDP route refused")
            Mockito.verify(native).disconnectPeer("listener", "Listener route authorization failed")
        } finally {
            controller.close()
        }
    }
}
