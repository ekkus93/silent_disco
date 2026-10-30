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

    private fun join() = FfiHostTransportEvent.JoinRequestReceived(
        deviceId = "listener-1",
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
            callDispatcher = dispatcher,
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
    fun failedNativeRefusalIsNotMisreportedAsDelivered() = runTest(dispatcher) {
        val native = Mockito.mock(FfiHostTransportHandle::class.java)
        Mockito.doAnswer { Thread.sleep(10); null }.`when`(native).pollEvent(500uL)
        Mockito.`when`(native.sendJoinRejection("listener-1", "host_busy"))
            .thenReturn(FfiHostTransportDelivery(1u, 0u, 1u, 0uL))
        val transport = HostTransportController(
            callDispatcher = dispatcher,
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
            repeat(200) {
                if (harness.viewModel.uiState.value.lastError?.contains("not delivered") == true) return@repeat
                Thread.sleep(5)
            }
            assertThat(harness.viewModel.uiState.value.lastError).contains("not delivered")
            assertThat(harness.viewModel.pendingListenerDatagramPorts.size).isEqualTo(0)
        } finally {
            transport.close()
        }
    }
}
