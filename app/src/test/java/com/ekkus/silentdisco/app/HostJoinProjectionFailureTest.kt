package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.rust.HostTransportController
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportDelivery
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportHandle
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

/** Exercises the real Android join-event projection and native refusal path. */
@OptIn(ExperimentalCoroutinesApi::class)
class HostJoinProjectionFailureTest {
    private val dispatcher = StandardTestDispatcher()

    @Before
    fun setUpMain() {
        Dispatchers.setMain(dispatcher)
    }

    @After
    fun resetMain() {
        Dispatchers.resetMain()
    }

    private fun join(deviceId: String = "listener-1") = FfiHostTransportEvent.JoinRequestReceived(
        deviceId = deviceId,
        displayName = "Unaccepted listener",
        inviteCode = null,
        syncPort = 101u,
        audioPort = 102u,
    )

    @Test
    fun failedActorProjectionRejectsOriginalTcpJoinAndReleasesItsRoute() = runTest(dispatcher) {
        val native = Mockito.mock(FfiHostTransportHandle::class.java)
        Mockito.doAnswer { Thread.sleep(10); null }.`when`(native).pollEvent(500uL)
        Mockito.`when`(native.sendJoinRejection("listener-1", "host_busy"))
            .thenReturn(FfiHostTransportDelivery(1u, 1u, 0u, 24uL))
        val transport = HostTransportController(
            // The refusal job runs on real IO; injecting a virtual dispatcher
            // here can park native delivery while Mockito verification blocks
            // the test thread that must advance virtual time.
            callDispatcher = Dispatchers.IO,
            bindHandle = { _, _, _, _, _ -> native },
        )
        val harness = newTestMainViewModelHarness(hostTransportController = transport)
        harness.hostCoreController.submitJoinRequestFailure = IllegalStateException("actor queue unavailable")
        try {
            transport.bind(backgroundScope, "127.0.0.1", 1, 2, 3, "session-1")
            harness.viewModel.handleHostTransportEvent(harness.hostCoreController, join(), native)

            Mockito.verify(native, Mockito.timeout(2_000)).sendJoinRejection("listener-1", "host_busy")
            assertThat(harness.hostCoreController.submitJoinRequestCalls).isEmpty()
            assertThat(harness.viewModel.pendingListenerDatagramPorts.size).isEqualTo(0)
            assertThat(harness.viewModel.uiState.value.lastError).contains("actor queue unavailable")
        } finally {
            transport.close()
        }
    }

    @Test
    fun fullPendingRouteTableRefusesTheNextAcceptedJoinWithoutSubmittingToActor() = runTest(dispatcher) {
        val native = Mockito.mock(FfiHostTransportHandle::class.java)
        Mockito.doAnswer { Thread.sleep(10); null }.`when`(native).pollEvent(500uL)
        Mockito.`when`(native.sendJoinRejection("overflow-listener", "host_busy"))
            .thenReturn(FfiHostTransportDelivery(1u, 1u, 0u, 24uL))
        val transport = HostTransportController(
            // The refusal job runs on real IO; injecting a virtual dispatcher
            // here can park native delivery while Mockito verification blocks
            // the test thread that must advance virtual time.
            callDispatcher = Dispatchers.IO,
            bindHandle = { _, _, _, _, _ -> native },
        )
        val harness = newTestMainViewModelHarness(hostTransportController = transport)
        try {
            transport.bind(backgroundScope, "127.0.0.1", 1, 2, 3, "session-1")
            repeat(128) { index ->
                harness.viewModel.handleHostTransportEvent(
                    harness.hostCoreController, join("listener-$index"), native,
                )
            }
            assertThat(harness.hostCoreController.submitJoinRequestCalls).hasSize(128)
            assertThat(harness.viewModel.pendingListenerDatagramPorts.size).isEqualTo(128)

            harness.viewModel.handleHostTransportEvent(
                harness.hostCoreController, join("overflow-listener"), native,
            )
            Mockito.verify(native, Mockito.timeout(2_000))
                .sendJoinRejection("overflow-listener", "host_busy")
            assertThat(harness.hostCoreController.submitJoinRequestCalls).hasSize(128)
            assertThat(harness.viewModel.pendingListenerDatagramPorts.size).isEqualTo(128)

            // A terminal event frees capacity; a subsequent distinct join
            // reaches the actor rather than remaining permanently refused.
            harness.viewModel.pendingListenerDatagramPorts.removeDevice("listener-0")
            harness.viewModel.handleHostTransportEvent(
                harness.hostCoreController, join("fresh-listener"), native,
            )
            assertThat(harness.hostCoreController.submitJoinRequestCalls).hasSize(129)
            assertThat(harness.viewModel.pendingListenerDatagramPorts.size).isEqualTo(128)
        } finally {
            transport.close()
        }
    }

    @Test
    fun failedNativeRefusalIsNotMisreportedAsDelivered() = runTest(dispatcher) {
        val native = Mockito.mock(FfiHostTransportHandle::class.java)
        Mockito.doAnswer { Thread.sleep(10); null }.`when`(native).pollEvent(500uL)
        Mockito.`when`(native.sendJoinRejection("listener-1", "host_busy"))
            .thenReturn(FfiHostTransportDelivery(1u, 0u, 1u, 0uL))
        val transport = HostTransportController(
            // The refusal job runs on real IO; injecting a virtual dispatcher
            // here can park native delivery while Mockito verification blocks
            // the test thread that must advance virtual time.
            callDispatcher = Dispatchers.IO,
            bindHandle = { _, _, _, _, _ -> native },
        )
        val harness = newTestMainViewModelHarness(hostTransportController = transport)
        harness.hostCoreController.submitJoinRequestFailure = IllegalStateException("actor queue unavailable")
        try {
            transport.bind(backgroundScope, "127.0.0.1", 1, 2, 3, "session-1")
            harness.viewModel.handleHostTransportEvent(harness.hostCoreController, join(), native)

            Mockito.verify(native, Mockito.timeout(2_000)).sendJoinRejection("listener-1", "host_busy")
            // Wait for the real IO coroutine to report its failed send to the
            // ViewModel rather than asserting on a pre-send UI snapshot.
            val deadline = System.nanoTime() + 2_000_000_000L
            while (harness.viewModel.uiState.value.lastError?.contains("not delivered") != true &&
                System.nanoTime() < deadline
            ) {
                Thread.sleep(5)
            }
            assertThat(harness.viewModel.uiState.value.lastError).contains("not delivered")
            assertThat(harness.viewModel.pendingListenerDatagramPorts.size).isEqualTo(0)
        } finally {
            transport.close()
        }
    }
}
