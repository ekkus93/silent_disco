package com.ekkus.silentdisco.app

import android.os.SystemClock
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.model.SyncState
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.rust.FakeListenerTransport
import com.ekkus.silentdisco.core.transport.TransportSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiListenerLifecycle
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiSyncConfidence
import com.ekkus.silentdisco.feature.listener.listenerPlaybackStatus
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

    @Test
    fun trackBoundaryPresentationWaitsForItsSampleTimeAndSurvivesPause() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        val viewModel = harness.viewModel
        viewModel.currentStreamId = StreamId("stream-1")
        viewModel.listenerStreamHostStartTimeMs = SystemClock.elapsedRealtime() + 1_000L
        viewModel._uiState.value = viewModel._uiState.value.copy(
            listenerState = ListenerLifecycleState.PLAYING,
            listenerPlaybackState = PlaybackState.PAUSED,
            listenerSyncState = SyncState(confidence = SyncQualityBadge.GOOD),
            listenerCurrentTrackName = "First track",
        )

        viewModel.handleListenerTransportEvent(
            harness.listenerCoreController,
            FfiListenerTransportEvent.TrackBoundary(
                streamId = "stream-1",
                firstSampleIndex = 0uL,
                entryId = "entry-2",
                displayName = "Second track",
            ),
        )
        runCurrent()
        assertThat(viewModel._uiState.value.listenerCurrentTrackName).isEqualTo("First track")
        assertThat(viewModel.pendingListenerTrackBoundaries).containsKey(0L)

        viewModel._uiState.value = viewModel._uiState.value.copy(listenerPlaybackState = PlaybackState.PLAYING)
        viewModel.rescheduleListenerTrackBoundaries()
        advanceTimeBy(999)
        runCurrent()
        assertThat(viewModel._uiState.value.listenerCurrentTrackName).isEqualTo("First track")

        advanceTimeBy(1)
        runCurrent()
        assertThat(viewModel._uiState.value.listenerCurrentTrackName).isEqualTo("Second track")
        assertThat(viewModel.pendingListenerTrackBoundaries).doesNotContainKey(0L)
    }

    @Test
    fun replayedStreamStartRefreshesTrackNameWhenReconnectingToSameStream() {
        assertThat(
            listenerTrackNameAfterStreamStart(
                isSameStream = true,
                currentTrackName = "Missed track boundary",
                announcedTrackName = "Current host track",
            ),
        ).isEqualTo("Current host track")
        assertThat(
            listenerTrackNameAfterStreamStart(
                isSameStream = true,
                currentTrackName = "Current track",
                announcedTrackName = " ",
            ),
        ).isEqualTo("Current track")
        assertThat(
            listenerTrackNameAfterStreamStart(
                isSameStream = false,
                currentTrackName = "Previous stream track",
                announcedTrackName = null,
            ),
        ).isNull()
    }

    @Test
    fun hostStopReturnsToApprovedAndCancelsTheRuntimeClockProbeLoop() = runTest(dispatcher) {
        val harness = bufferingListener()
        harness.viewModel.listenerTransportConnected = true
        harness.viewModel._uiState.value = harness.viewModel._uiState.value.copy(
            listenerState = ListenerLifecycleState.PLAYING,
            listenerPlaybackState = PlaybackState.PLAYING,
        )
        harness.viewModel.startPeriodicListenerResync()
        runCurrent()
        assertThat(harness.viewModel.resyncJob?.isActive).isTrue()

        harness.viewModel.handleTransportStreamStopped()
        runCurrent()

        assertThat(harness.viewModel._uiState.value.listenerState)
            .isEqualTo(ListenerLifecycleState.APPROVED)
        assertThat(harness.viewModel._uiState.value.listenerPlaybackState)
            .isEqualTo(PlaybackState.STOPPED)
        assertThat(harness.viewModel._uiState.value.connectionProgress.currentState)
            .isEqualTo(ListenerLifecycleState.APPROVED)
        assertThat(harness.viewModel.resyncJob).isNull()
        assertThat(listenerPlaybackStatus(harness.viewModel._uiState.value))
            .isEqualTo("Playback stopped")
    }

    @Test
    fun syncAcquisitionFailureClearsTheOldClockMappingBeforeRetry() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.ensureRustListenerCore()
        harness.viewModel.pendingSyncCorrelationId = 47L
        harness.viewModel._uiState.value = harness.viewModel._uiState.value.copy(
            listenerSyncState = SyncState(
                offsetMs = -15_100_405.5,
                rttMs = 93.0,
                confidence = SyncQualityBadge.POOR,
                resyncCount = 3,
            ),
            connectionProgress = harness.viewModel._uiState.value.connectionProgress.copy(synced = true),
        )

        harness.viewModel.handleSyncFailure("Unable to establish a stable sync estimate")

        assertThat(harness.viewModel.pendingSyncCorrelationId).isNull()
        assertThat(harness.viewModel.listenerSyncEstimator).isNull()
        assertThat(harness.viewModel._uiState.value.listenerSyncState.offsetMs).isEqualTo(0.0)
        assertThat(harness.viewModel._uiState.value.listenerSyncState.confidence)
            .isEqualTo(SyncQualityBadge.UNKNOWN)
        assertThat(harness.viewModel._uiState.value.listenerSyncState.resyncCount).isEqualTo(3)
        assertThat(harness.viewModel._uiState.value.connectionProgress.synced).isFalse()
        assertThat(harness.viewModel._uiState.value.lastError)
            .isEqualTo("Couldn't establish a steady connection. Try reconnecting closer to the host.")
        assertThat(harness.listenerCoreController.transportFailedCalls)
            .containsExactly(
                "Couldn't establish a steady connection. Try reconnecting closer to the host." to true,
            )
    }

    @Test
    fun cleanHostCloseLeavesTheSessionWithoutShowingAConnectionFailure() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.ensureRustListenerCore()
        harness.viewModel._uiState.value = harness.viewModel._uiState.value.copy(
            selectedSession = SessionInfo(
                id = "session-1",
                name = "Patio Mix",
                hostDeviceName = "desktop-host",
                approvalMode = ApprovalMode.MANUAL,
                inviteCodeRequired = false,
            ),
            listenerState = ListenerLifecycleState.PLAYING,
            listenerPlaybackState = PlaybackState.PLAYING,
        )

        harness.viewModel.handleListenerTransportEvent(
            harness.listenerCoreController,
            FfiListenerTransportEvent.ConnectionClosed(message = null),
        )

        assertThat(harness.listenerCoreController.cancelJoinCallCount).isEqualTo(1)
        assertThat(harness.listenerCoreController.transportFailedCalls).isEmpty()
        assertThat(harness.viewModel._uiState.value.listenerPlaybackState).isEqualTo(PlaybackState.STOPPED)
    }

    @Test
    fun discoveredListenerRelaysItsSyncSummaryToTheHostTransport() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()

        harness.viewModel.reportListenerSyncToHost(
            FfiSyncConfidence.EXCELLENT,
            offsetMs = 72_401.0,
            roundTripMs = 12.0,
            driftPpm = 3.5,
        )

        assertThat(harness.listenerTransport.syncReportCalls).containsExactly(
            FakeListenerTransport.SyncReportCall(
                FfiSyncConfidence.EXCELLENT,
                72_401.0,
                12.0,
                3.5,
            ),
        )
    }

    @Test
    fun wifiDirectCleanupIsLimitedToActiveOrPendingWifiDirectJoins() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()

        assertThat(harness.viewModel.listenerHasPendingWifiDirectJoin).isFalse()

        harness.viewModel.pendingEstablishNetworkOperationId = "pending-wifi-direct-join"
        assertThat(harness.viewModel.listenerHasPendingWifiDirectJoin).isTrue()

        harness.viewModel.pendingEstablishNetworkOperationId = null
        harness.viewModel.listenerTransportRidesWifiDirect = true
        assertThat(harness.viewModel.listenerHasPendingWifiDirectJoin).isTrue()
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
