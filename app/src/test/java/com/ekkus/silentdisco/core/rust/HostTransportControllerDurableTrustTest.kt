package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.test.runTest
import org.junit.Test

/** The Android host cannot remember listeners yet: "Always allow" is refused, approve-once is not. */
class HostTransportControllerDurableTrustTest {
    @Test
    fun directApprovalRejectsDurableTrustEvenBeforeNativeBind() = runTest {
        val controller = HostTransportController()
        try {
            val failure = runCatching {
                controller.sendJoinApproval("claimed-listener", trustedForFuture = true)
            }.exceptionOrNull()
            assertThat(failure).isInstanceOf(IllegalStateException::class.java)
            assertThat(failure?.message).isEqualTo(DURABLE_LISTENER_TRUST_UNAVAILABLE)
        } finally {
            controller.close()
        }
    }

    @Test
    fun combinedApprovalAndAuthorizationCannotRememberTheListener() = runTest {
        val controller = HostTransportController()
        try {
            val failure = runCatching {
                controller.approveAndAuthorizeListener(
                    "claimed-listener",
                    trustedForFuture = true,
                    syncPort = 11001u,
                    audioPort = 11002u,
                )
            }.exceptionOrNull()
            assertThat(failure).isInstanceOf(IllegalStateException::class.java)
            assertThat(failure?.message).isEqualTo(DURABLE_LISTENER_TRUST_UNAVAILABLE)
            // Manual one-session admission is still permitted by policy;
            // absent a bound native session it cannot send an approval.
            assertThat(
                controller.approveAndAuthorizeListener(
                    "claimed-listener",
                    trustedForFuture = false,
                    syncPort = 11001u,
                    audioPort = 11002u,
                ),
            ).isNull()
        } finally {
            controller.close()
        }
    }
}
