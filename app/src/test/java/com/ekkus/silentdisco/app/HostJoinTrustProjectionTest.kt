package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.rust.HostTransportController
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportHandle
import com.ekkus.silentdisco.core.uniffi.FfiTrustState
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.mockito.Mockito

/**
 * F1 reproduction at the Android production join boundary. A peer-supplied
 * device ID is only a claim: the host must hand it to the Rust actor as
 * [FfiTrustState.UNKNOWN], never as trusted, so the Rust "Approved devices
 * only" policy can refuse it. Verified identity is a separate security gate.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class HostJoinTrustProjectionTest {
    private val dispatcher = StandardTestDispatcher()

    @Before
    fun setUpMain() {
        Dispatchers.setMain(dispatcher)
    }

    @After
    fun resetMain() {
        Dispatchers.resetMain()
    }

    @Test
    fun claimedDeviceIdReachesTheActorAsUnknownTrust() = runTest(dispatcher) {
        val native = Mockito.mock(FfiHostTransportHandle::class.java)
        Mockito.doAnswer { Thread.sleep(10); null }.`when`(native).pollEvent(500uL)
        val transport = HostTransportController(
            callDispatcher = Dispatchers.IO,
            bindHandle = { _, _, _, _, _ -> native },
        )
        val harness = newTestMainViewModelHarness(hostTransportController = transport)
        try {
            transport.bind(backgroundScope, "127.0.0.1", 1, 2, 3, "session-1")
            harness.viewModel.handleHostTransportEvent(
                harness.hostCoreController,
                FfiHostTransportEvent.JoinRequestReceived(
                    controlAddress = "127.0.0.1:10001",
                    deviceId = "previously-approved-device",
                    displayName = "Claimed identity",
                    inviteCode = null,
                    syncPort = 101u,
                    audioPort = 102u,
                ),
                native,
            )

            val submitted = harness.hostCoreController.submitJoinRequestCalls
            assertThat(submitted).hasSize(1)
            assertThat(submitted.single().deviceId).isEqualTo("previously-approved-device")
            assertThat(submitted.single().trustState).isEqualTo(FfiTrustState.UNKNOWN)
        } finally {
            transport.close()
        }
    }
}
