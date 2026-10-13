package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.transport.TransportSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiListenerLifecycle
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Before
import org.junit.Test

/**
 * Block 52.6: the periodic resync loop treated Wi-Fi Direct's state as the
 * session's liveness. A desktop host joined via mDNS or a manual endpoint never
 * uses Wi-Fi Direct, so a disconnected/idle Wi-Fi Direct service would end a
 * healthy session the moment the loop ran.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class MainViewModelListenerResyncTest {
    private val dispatcher = StandardTestDispatcher()

    @Before
    fun setUp() {
        Dispatchers.setMain(dispatcher)
    }

    @After
    fun tearDown() {
        Dispatchers.resetMain()
    }

    @Test
    fun wifiDirectStateDoesNotEndASessionThatDoesNotRideWifiDirect() = runTest(dispatcher) {
        val harness = bufferingListener()
        harness.viewModel.listenerTransportRidesWifiDirect = false
        harness.wifiDirectService.emitSnapshot(
            TransportSnapshot(state = TransportConnectionState.DISCONNECTED),
        )

        harness.viewModel.startPeriodicListenerResync()
        advanceTimeBy(2_000)
        runCurrent()

        assertThat(harness.listenerCoreController.transportFailedCalls).isEmpty()
        assertThat(harness.viewModel.resyncJob?.isActive).isTrue()
        harness.viewModel.resyncJob?.cancel()
    }

    @Test
    fun wifiDirectDisconnectStillEndsASessionThatRidesWifiDirect() = runTest(dispatcher) {
        val harness = bufferingListener()
        harness.viewModel.listenerTransportRidesWifiDirect = true
        harness.wifiDirectService.emitSnapshot(
            TransportSnapshot(state = TransportConnectionState.DISCONNECTED),
        )

        harness.viewModel.startPeriodicListenerResync()
        advanceTimeBy(2_000)
        runCurrent()

        assertThat(harness.listenerCoreController.transportFailedCalls)
            .containsExactly("Transport disconnected during playback" to true)
    }

    @Test
    fun approvalStartsClockProbesBeforePlaybackRuntimeExists() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.ensureRustListenerCore()
        harness.viewModel.listenerTransportConnected = true

        harness.listenerCoreController.publishSnapshot(
            listenerCoreSnapshot(
                discoveryActive = false,
                known = listOf("session-1"),
                selected = "session-1",
                listenerLifecycle = FfiListenerLifecycle.APPROVED,
            ),
        )
        runCurrent()
        assertThat(harness.viewModel.listenerPlayback).isNull()

        advanceTimeBy(250)
        runCurrent()

        assertThat(harness.listenerTransport.syncRequestCalls).hasSize(1)
        assertThat(harness.viewModel._uiState.value.listenerState)
            .isEqualTo(ListenerLifecycleState.SYNCING_CLOCK)
        harness.viewModel.resyncJob?.cancel()
    }

    @Test
    fun pausedPlaybackKeepsTheListenerSyncProbeLoopActive() = runTest(dispatcher) {
        val harness = bufferingListener()
        harness.viewModel.listenerTransportConnected = true
        harness.viewModel.propagateListenerPlaybackState(
            playbackState = PlaybackState.PAUSED,
            listenerState = harness.viewModel._uiState.value.listenerState,
            message = "Host paused the stream",
        )
        harness.viewModel.startPeriodicListenerResync()

        advanceTimeBy(250)
        runCurrent()

        assertThat(harness.listenerTransport.syncRequestCalls).hasSize(1)
        assertThat(harness.viewModel._uiState.value.listenerPlaybackState)
            .isEqualTo(PlaybackState.PAUSED)
        harness.viewModel.resyncJob?.cancel()
    }

    private fun kotlinx.coroutines.test.TestScope.bufferingListener(): MainViewModelHarness {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.ensureRustListenerCore()
        val session = SessionInfo(
            id = "session-1",
            name = "Patio Mix",
            hostDeviceName = "desktop-host",
            approvalMode = ApprovalMode.MANUAL,
            inviteCodeRequired = false,
        )
        harness.viewModel._uiState.value = harness.viewModel._uiState.value.copy(
            selectedSession = session,
            listenerState = ListenerLifecycleState.BUFFERING,
        )
        runCurrent()
        return harness
    }
}
