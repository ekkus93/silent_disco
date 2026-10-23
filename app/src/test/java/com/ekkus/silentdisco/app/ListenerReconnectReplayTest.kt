package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.model.SyncState
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiPlaybackPhase
import com.ekkus.silentdisco.core.uniffi.NoHandle
import com.google.common.truth.Truth.assertThat
import java.util.Collections
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Before
import org.junit.Test

/**
 * Drives the production listener transport-event handler through the control
 * replay a host's transport cache sends to a reconnecting listener:
 * `StreamStart`, every cached `TrackBoundary`, then `Pause` when paused.
 *
 * The JVM `SystemClock` reads 0, so local "now" is 0 and a positive host
 * offset of [HOST_OFFSET_MS] maps host time `h` to local time `h - 10 000`.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class ListenerReconnectReplayTest {
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
    fun pausedReplayDoesNotAdvanceToABoundaryAfterThePausePoint() = runTest(dispatcher) {
        val fixture = reconnectingListener(pendingBeforeDisconnect = boundary(FUTURE_SAMPLE, "Third track"))

        // The replayed StreamStart still carries the pre-pause anchor, so the
        // future boundary's local time is already past when it is replayed.
        fixture.deliver(streamStarted(hostStartTimeMs = 1_000uL, title = "First track"))
        fixture.deliver(boundary(PAST_SAMPLE, "Second track"))
        fixture.deliver(boundary(FUTURE_SAMPLE, "Third track"))
        fixture.deliver(FfiListenerTransportEvent.Paused(STREAM_ID, hostPauseTimeMs = 1_500uL))

        assertThat(fixture.viewModel._uiState.value.listenerCurrentTrackName).isEqualTo("Second track")
        assertThat(fixture.viewModel.pendingListenerTrackBoundaries.keys).containsExactly(FUTURE_SAMPLE)
        assertThat(fixture.viewModel._uiState.value.listenerPlaybackState).isEqualTo(PlaybackState.PAUSED)
    }

    @Test
    fun resumeAfterPausedReplayAppliesTheHeldBoundaryAtItsShiftedTime() = runTest(dispatcher) {
        val fixture = reconnectingListener(pendingBeforeDisconnect = null)
        fixture.deliver(streamStarted(hostStartTimeMs = 1_000uL, title = "First track"))
        fixture.deliver(boundary(PAST_SAMPLE, "Second track"))
        fixture.deliver(boundary(FUTURE_SAMPLE, "Third track"))
        fixture.deliver(FfiListenerTransportEvent.Paused(STREAM_ID, hostPauseTimeMs = 1_500uL))

        // A 60 s pause: the host re-anchors and replays the still-pending boundary.
        fixture.deliver(streamStarted(hostStartTimeMs = 61_000uL, title = "Second track"))
        fixture.deliver(boundary(FUTURE_SAMPLE, "Third track"))
        assertThat(fixture.runtime.reanchoredStartsMs).containsExactly(1_000uL, 61_000uL).inOrder()
        assertThat(fixture.viewModel._uiState.value.listenerPlaybackState).isEqualTo(PlaybackState.BUFFERING)

        // Host time 61 000 + 1 000 = 62 000 maps to local 52 000.
        advanceTimeBy(51_999L)
        runCurrent()
        assertThat(fixture.viewModel._uiState.value.listenerCurrentTrackName).isEqualTo("Second track")

        advanceTimeBy(1L)
        runCurrent()
        assertThat(fixture.viewModel._uiState.value.listenerCurrentTrackName).isEqualTo("Third track")
        assertThat(fixture.viewModel.pendingListenerTrackBoundaries).isEmpty()
    }

    @Test
    fun playingReplayAppliesEveryPassedBoundaryInOrderAndHoldsTheNextOne() = runTest(dispatcher) {
        val fixture = reconnectingListener(pendingBeforeDisconnect = boundary(PAST_SAMPLE, "Second track"))

        // Reconnecting while the host plays at host time 10 000 + now: the
        // track-two boundary passed during the outage; track three is ahead.
        fixture.deliver(streamStarted(hostStartTimeMs = 9_000uL, title = "First track"))
        fixture.deliver(boundary(PAST_SAMPLE, "Second track"))
        fixture.deliver(boundary(LATER_SAMPLE, "Third track"))

        assertThat(fixture.viewModel._uiState.value.listenerCurrentTrackName).isEqualTo("Second track")
        assertThat(fixture.viewModel.pendingListenerTrackBoundaries.keys).containsExactly(LATER_SAMPLE)

        // Host 9 000 + 5 000 = 14 000 maps to local 4 000.
        advanceTimeBy(4_000L)
        runCurrent()
        assertThat(fixture.viewModel._uiState.value.listenerCurrentTrackName).isEqualTo("Third track")
    }


    @Test
    fun duplicateCachedBoundariesNeverScheduleDuplicateOrRewindCurrentTrack() = runTest(dispatcher) {
        val fixture = reconnectingListener(pendingBeforeDisconnect = null)
        fixture.deliver(streamStarted(hostStartTimeMs = 9_000uL, title = "First track"))
        fixture.deliver(boundary(PAST_SAMPLE, "Second track"))
        fixture.deliver(boundary(PAST_SAMPLE, "Second track"))
        fixture.deliver(boundary(LATER_SAMPLE, "Third track"))
        fixture.deliver(boundary(LATER_SAMPLE, "Third track"))

        assertThat(fixture.viewModel._uiState.value.listenerCurrentTrackName).isEqualTo("Second track")
        assertThat(fixture.viewModel.pendingListenerTrackBoundaries.keys).containsExactly(LATER_SAMPLE)
        assertThat(fixture.viewModel.listenerTrackBoundaryJobs.keys).containsExactly(LATER_SAMPLE)

        advanceTimeBy(3_999L)
        runCurrent()
        assertThat(fixture.viewModel._uiState.value.listenerCurrentTrackName).isEqualTo("Second track")
        advanceTimeBy(1L)
        runCurrent()
        assertThat(fixture.viewModel._uiState.value.listenerCurrentTrackName).isEqualTo("Third track")
        assertThat(fixture.viewModel.pendingListenerTrackBoundaries).isEmpty()
    }

    @Test
    fun runtimePollPromotesOnlyBufferingToPlaying() {
        assertThat(shouldReportListenerPlaying(FfiPlaybackPhase.PLAYING, PlaybackState.BUFFERING)).isTrue()
        assertThat(shouldReportListenerPlaying(FfiPlaybackPhase.PLAYING, PlaybackState.PAUSED)).isFalse()
        assertThat(shouldReportListenerPlaying(FfiPlaybackPhase.PLAYING, PlaybackState.PLAYING)).isFalse()
        assertThat(shouldReportListenerPlaying(FfiPlaybackPhase.BUFFERING, PlaybackState.BUFFERING)).isFalse()
    }

    private fun TestScope.reconnectingListener(
        pendingBeforeDisconnect: FfiListenerTransportEvent.TrackBoundary?,
    ): ReplayFixture {
        val harness = newTestMainViewModelHarness()
        val viewModel = harness.viewModel
        viewModel.ensureRustListenerCore()
        val runtime = RecordingListenerPlayback()
        // The listener was already following this stream when its connection
        // dropped; the runtime survives so the replay re-anchors it in place.
        viewModel.currentStreamId = StreamId(STREAM_ID)
        viewModel.listenerPlayback = runtime
        viewModel._uiState.value = viewModel._uiState.value.copy(
            selectedSession = SessionInfo(
                id = "session-1",
                name = "Patio Mix",
                hostDeviceName = "desktop-host",
                approvalMode = ApprovalMode.MANUAL,
                inviteCodeRequired = false,
            ),
            listenerState = ListenerLifecycleState.PLAYING,
            listenerPlaybackState = PlaybackState.PLAYING,
            listenerSyncState = SyncState(offsetMs = HOST_OFFSET_MS, confidence = SyncQualityBadge.GOOD),
            listenerCurrentTrackName = "First track",
        )
        pendingBeforeDisconnect?.let { pending ->
            viewModel.pendingListenerTrackBoundaries[pending.firstSampleIndex.toLong()] =
                PendingListenerTrackBoundary(pending.streamId, pending.firstSampleIndex.toLong(), pending.displayName)
        }
        runCurrent()
        return ReplayFixture(this, harness, runtime)
    }

    private class ReplayFixture(
        private val scope: TestScope,
        val harness: MainViewModelHarness,
        val runtime: RecordingListenerPlayback,
    ) {
        val viewModel: MainViewModel get() = harness.viewModel

        /** Handles one event the way the transport collector does, then lets its immediate work run. */
        fun deliver(event: FfiListenerTransportEvent) {
            viewModel.handleListenerTransportEvent(harness.listenerCoreController, event)
            viewModel.listenerPlaybackControlExecutor.submit {}.get()
            scope.runCurrent()
        }
    }

    private fun streamStarted(hostStartTimeMs: ULong, title: String) = FfiListenerTransportEvent.StreamStarted(
        streamId = STREAM_ID,
        hostStartTimeMs = hostStartTimeMs,
        sampleRate = 48_000u,
        channels = 2u,
        samplesPerPacket = 960u,
        currentEntryId = "entry-current",
        currentDisplayName = title,
    )

    private fun boundary(sampleIndex: Long, title: String) = FfiListenerTransportEvent.TrackBoundary(
        streamId = STREAM_ID,
        firstSampleIndex = sampleIndex.toULong(),
        entryId = "entry-$sampleIndex",
        displayName = title,
    )

    private companion object {
        const val STREAM_ID = "replay-stream"
        const val HOST_OFFSET_MS = 10_000.0

        /** Host time 1 000 + 250 = 1 250 ms: before the pause at 1 500. */
        const val PAST_SAMPLE = 12_000L

        /** Host time 1 000 + 1 000 = 2 000 ms: after the pause at 1 500. */
        const val FUTURE_SAMPLE = 48_000L

        /** 5 s into the stream. */
        const val LATER_SAMPLE = 240_000L
    }
}

/** Accepts re-anchors without a native runtime. */
private class RecordingListenerPlayback : FfiListenerPlaybackHandle(NoHandle) {
    val reanchoredStartsMs: MutableList<ULong> = Collections.synchronizedList(mutableListOf())

    override fun reanchorPresentationTime(hostStartTimeMs: ULong) {
        reanchoredStartsMs += hostStartTimeMs
    }
}
