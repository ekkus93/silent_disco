package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.test.runTest
import org.junit.Test

/** Legacy-v3 admission must not turn self-declared IDs into durable trusted identities. */
class HostTransportControllerLegacyTrustTest {
    @Test
    fun directApprovalRejectsDurableTrustEvenBeforeNativeBind() = runTest {
        val controller = HostTransportController()
        try {
            val failure = runCatching {
                controller.sendJoinApproval("claimed-listener", trustedForFuture = true)
            }.exceptionOrNull()
            assertThat(failure).isInstanceOf(IllegalStateException::class.java)
            assertThat(failure?.message).isEqualTo(LEGACY_V3_DURABLE_TRUST_ERROR)
        } finally {
            controller.close()
        }
    }

    @Test
    fun combinedApprovalAndAuthorizationCannotPromoteLegacyClaimedId() = runTest {
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
            assertThat(failure?.message).isEqualTo(LEGACY_V3_DURABLE_TRUST_ERROR)
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
