package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.test.runTest
import org.junit.Test

/**
 * "Always allow" is now stored by Rust before the approval is sent
 * (remediation F1), so the transport no longer refuses it. Without a bound
 * native session neither kind of approval can be sent.
 */
class HostTransportControllerDurableTrustTest {
    @Test
    fun anUnboundHostSendsNoApprovalOfEitherKind() = runTest {
        val controller = HostTransportController()
        try {
            assertThat(controller.sendJoinApproval("v4-listener", trustedForFuture = true)).isNull()
            for (trusted in listOf(true, false)) {
                assertThat(
                    controller.approveAndAuthorizeListener(
                        "v4-listener",
                        trustedForFuture = trusted,
                        syncPort = 11001u,
                        audioPort = 11002u,
                    ),
                ).isNull()
            }
        } finally {
            controller.close()
        }
    }
}
