package com.ekkus.silentdisco.app

import com.google.common.truth.Truth.assertThat
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.uniffi.FfiHostBoundary
import com.ekkus.silentdisco.core.uniffi.FfiHostPlaylistException
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistStreamStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Before
import org.junit.Test

/**
 * The Android host's boundary poller (gapless TODO §5): Rust's schedule
 * decides when a boundary is due; Kotlin supplies the host clock and the
 * stream loop's pause offset, waits while a pause is being applied, and
 * applies what comes back to the host core and the stream announcement.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class HostQueueBoundarySchedulerTest {
    private val dispatcher = StandardTestDispatcher()

    @Before
    fun setUpMainDispatcher() {
        Dispatchers.setMain(dispatcher)
    }

    @After
    fun resetMainDispatcher() {
        Dispatchers.resetMain()
    }

    private fun TestScope.startedHost(): Pair<MainViewModelHarness, FakeHostBoundarySchedule> {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.hostPlayback.streamStart = FfiPlaylistStreamStart(
            streamId = "stream-1",
            hostStartTimeMs = 100uL,
            sampleRate = 1_000u,
            channels = 2u,
            samplesPerPacket = 20u,
            currentEntryId = "entry-1",
            currentDisplayName = "First track",
        )
        harness.viewModel.hostPlayback.openBoundarySchedule(100uL, 1_000u)
        return harness to harness.boundarySchedules.latest
    }

    private fun MainViewModelHarness.runPoller(scope: TestScope) =
        viewModel.runHostBoundaryPoller(
            dispatcher = Dispatchers.Main,
            elapsedRealtimeMs = { scope.testScheduler.currentTime },
        )

    @Test
    fun theBoundaryIsAppliedAtThePauseAdjustedDeadlineAndAdvancesTheAnnouncedTrack() = runTest(dispatcher) {
        val (harness, schedule) = startedHost()
        val viewModel = harness.viewModel
        schedule.schedule("entry-2", 100uL, "Second track") // due at 200 ms before any pause
        harness.runPoller(this)
        runCurrent()
        advanceTimeBy(40L)
        viewModel._uiState.value = viewModel._uiState.value.copy(hostPlaybackState = PlaybackState.PAUSED)
        viewModel.hostPlayback.pauseInProgress = true
        runCurrent()
        advanceTimeBy(60L)
        // What the stream loop does on resume: apply the pause to the audio.
        viewModel.hostPlayback.presentationOffsetMs = 60L
        viewModel.hostPlayback.pauseInProgress = false
        viewModel._uiState.value = viewModel._uiState.value.copy(hostPlaybackState = PlaybackState.PLAYING)
        runCurrent()

        advanceTimeBy(159L)
        runCurrent()
        assertThat(harness.hostCoreController.trackBoundaryAppliedCalls).isEmpty()

        advanceTimeBy(1L)
        runCurrent()
        assertThat(harness.hostCoreController.trackBoundaryAppliedCalls).containsExactly("entry-2" to 100L)
        assertThat(viewModel.hostPlayback.streamStart?.currentEntryId).isEqualTo("entry-2")
        assertThat(viewModel.hostPlayback.streamStart?.currentDisplayName).isEqualTo("Second track")
        viewModel.cancelHostBoundaryJobs()
    }

    /**
     * Kotlin passes Rust the offset the stream loop applied to the audio,
     * never a pause it measured itself, and asks nothing while a pause is in
     * progress (it would be measured twice; gapless TODO §4).
     */
    @Test
    fun thePollerPassesTheStreamLoopsPauseOffsetAndDoesNotAskWhilePaused() = runTest(dispatcher) {
        val (harness, schedule) = startedHost()
        val viewModel = harness.viewModel
        schedule.schedule("entry-2", 1_000uL, "Second track")
        harness.runPoller(this)
        runCurrent()
        advanceTimeBy(100L)
        viewModel.hostPlayback.pauseInProgress = true
        runCurrent()
        val askedBeforePause = schedule.takeDueCalls.size
        advanceTimeBy(300L)
        runCurrent()
        assertThat(schedule.takeDueCalls).hasSize(askedBeforePause)

        viewModel.hostPlayback.presentationOffsetMs = 300L
        viewModel.hostPlayback.pauseInProgress = false
        advanceTimeBy(20L)
        runCurrent()
        assertThat(schedule.takeDueCalls.drop(askedBeforePause).map { it.second }.toSet()).containsExactly(300L)
        assertThat(schedule.takeDueCalls.first().second).isEqualTo(0L)
        assertThat(schedule.takeDueCalls.all { (now, _) -> now <= testScheduler.currentTime }).isTrue()
        viewModel.cancelHostBoundaryJobs()
    }

    @Test
    fun boundariesOverdueTogetherAreAppliedInSampleOrder() = runTest(dispatcher) {
        val (harness, schedule) = startedHost()
        schedule.schedule("entry-2", 50uL, "Second")
        schedule.schedule("entry-3", 60uL, "Third")
        // The poller starts late: both are already due.
        advanceTimeBy(500L)
        harness.runPoller(this)
        runCurrent()
        assertThat(harness.hostCoreController.trackBoundaryAppliedCalls)
            .containsExactly("entry-2" to 50L, "entry-3" to 60L).inOrder()
        assertThat(harness.viewModel.hostPlayback.streamStart?.currentEntryId).isEqualTo("entry-3")
        harness.viewModel.cancelHostBoundaryJobs()
    }

    @Test
    fun aBoundaryIsNotAppliedAfterStopCancelsThePoller() = runTest(dispatcher) {
        val (harness, schedule) = startedHost()
        schedule.schedule("entry-2", 100uL, "Second track")
        harness.runPoller(this)
        runCurrent()
        harness.viewModel.cancelHostBoundaryJobs()
        advanceUntilIdle()

        assertThat(harness.hostCoreController.trackBoundaryAppliedCalls).isEmpty()
        assertThat(harness.viewModel.hostPlayback.boundaryPoller).isNull()
    }

    @Test
    fun aRustScheduleFailureIsShownAndStopsThePoller() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        val failing = object : FakeHostBoundarySchedule(100uL, 1_000u) {
            override fun takeDue(nowMs: ULong, pauseOffsetMs: ULong): List<FfiHostBoundary> =
                throw FfiHostPlaylistException.Closed("host boundary schedule is poisoned")
        }
        harness.boundarySchedules.next = failing
        harness.viewModel.hostPlayback.openBoundarySchedule(100uL, 1_000u)
        harness.runPoller(this)
        advanceUntilIdle()

        assertThat(harness.viewModel.uiState.value.lastError).isEqualTo("host boundary schedule is poisoned")
        assertThat(harness.viewModel.hostPlayback.boundaryPoller?.isActive ?: false).isFalse()
    }

    @Test
    fun aPollerForAStreamWithoutAScheduleEndsAndTheNextStreamStartsAFreshOne() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.runPoller(this)
        advanceUntilIdle()
        assertThat(harness.viewModel.hostPlayback.boundaryPoller?.isActive ?: false).isFalse()

        harness.viewModel.hostPlayback.openBoundarySchedule(0uL, 1_000u)
        harness.boundarySchedules.latest.schedule("entry-2", 10uL, "Second")
        harness.runPoller(this)
        advanceTimeBy(10L)
        runCurrent()
        assertThat(harness.hostCoreController.trackBoundaryAppliedCalls).containsExactly("entry-2" to 10L)
        harness.viewModel.cancelHostBoundaryJobs()
    }
}
