package com.ekkus.silentdisco.app

import com.google.common.truth.Truth.assertThat
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistFrame
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistStreamStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Before
import org.junit.Test

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

    @Test
    fun pauseShiftsScheduledBoundaryByThePausedDuration() = runTest(dispatcher) {
        val scheduler = testScheduler
        var appliedAtMs: Long? = null
        launch {
            appliedAtMs = awaitHostQueueBoundary(
                dueAtElapsedRealtimeMs = 100L,
                isPaused = { scheduler.currentTime in 40L until 100L },
                // The stream loop applies the 60 ms pause when it resumes.
                pauseOffsetMs = { if (scheduler.currentTime >= 100L) 60L else 0L },
                elapsedRealtimeMs = { scheduler.currentTime },
            )
        }

        runCurrent()
        advanceTimeBy(159L)
        runCurrent()
        assertThat(appliedAtMs).isNull()

        advanceTimeBy(1L)
        runCurrent()
        assertThat(appliedAtMs).isEqualTo(160L)
    }

    /**
     * The timer follows the pause offset the stream loop applied to the audio,
     * not its own view of the pause: here the loop's polls timed 80 ms where
     * the timer saw a 60 ms pause, and the boundary moves by 80 ms, with the
     * audio (gapless TODO §4: one pause measurement, as on the desktop).
     */
    @Test
    fun theBoundaryFollowsTheStreamLoopsPauseOffsetNotItsOwnPollOfThePause() = runTest(dispatcher) {
        val scheduler = testScheduler
        var appliedAtMs: Long? = null
        launch {
            appliedAtMs = awaitHostQueueBoundary(
                dueAtElapsedRealtimeMs = 100L,
                isPaused = { scheduler.currentTime in 40L until 100L },
                pauseOffsetMs = { if (scheduler.currentTime >= 100L) 80L else 0L },
                elapsedRealtimeMs = { scheduler.currentTime },
            )
        }

        runCurrent()
        advanceTimeBy(179L)
        runCurrent()
        assertThat(appliedAtMs).isNull()

        advanceTimeBy(1L)
        runCurrent()
        assertThat(appliedAtMs).isEqualTo(180L)
    }

    @Test
    fun aBoundaryWaitsWhileTheStreamLoopHasNotYetAppliedThePause() = runTest(dispatcher) {
        val scheduler = testScheduler
        var appliedAtMs: Long? = null
        // The UI resumed at 100 ms, but the loop applies the pause only at
        // 115 ms; until then the timer must not use the old offset.
        launch {
            appliedAtMs = awaitHostQueueBoundary(
                dueAtElapsedRealtimeMs = 100L,
                isPaused = { scheduler.currentTime in 40L until 115L },
                pauseOffsetMs = { if (scheduler.currentTime >= 115L) 75L else 0L },
                elapsedRealtimeMs = { scheduler.currentTime },
            )
        }

        runCurrent()
        advanceTimeBy(174L)
        runCurrent()
        assertThat(appliedAtMs).isNull()

        advanceTimeBy(1L)
        runCurrent()
        assertThat(appliedAtMs).isEqualTo(175L)
    }

    @Test
    fun productionHostSchedulerAppliesBoundaryAtPauseAdjustedDeadline() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        val start = FfiPlaylistStreamStart(
            streamId = "stream-1",
            hostStartTimeMs = 100uL,
            sampleRate = 1_000u,
            channels = 2u,
            samplesPerPacket = 20u,
            currentEntryId = "entry-1",
            currentDisplayName = "First track",
        )
        val boundary = FfiPlaylistFrame.TrackBoundary(
            streamId = "stream-1",
            firstSampleIndex = 100uL,
            entryId = "entry-2",
            displayName = "Second track",
        )
        val viewModel = harness.viewModel
        viewModel.hostPlayback.streamStart = start

        viewModel.scheduleHostQueueBoundary(
            start = start,
            boundary = boundary,
            sampleRate = 1_000,
            dispatcher = Dispatchers.Main,
            elapsedRealtimeMs = { testScheduler.currentTime },
        )
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
        assertThat(harness.hostCoreController.trackBoundaryAppliedCalls)
            .containsExactly("entry-2" to 100L)
        assertThat(viewModel.hostPlayback.streamStart?.currentEntryId).isEqualTo("entry-2")
        assertThat(viewModel.hostPlayback.streamStart?.currentDisplayName).isEqualTo("Second track")
    }

    @Test
    fun productionHostSchedulerDoesNotApplyBoundaryAfterStopCancelsIt() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        val start = FfiPlaylistStreamStart(
            streamId = "stream-1",
            hostStartTimeMs = 100uL,
            sampleRate = 1_000u,
            channels = 2u,
            samplesPerPacket = 20u,
            currentEntryId = "entry-1",
            currentDisplayName = "First track",
        )
        val boundary = FfiPlaylistFrame.TrackBoundary(
            streamId = "stream-1",
            firstSampleIndex = 100uL,
            entryId = "entry-2",
            displayName = "Second track",
        )

        harness.viewModel.scheduleHostQueueBoundary(
            start = start,
            boundary = boundary,
            sampleRate = 1_000,
            dispatcher = Dispatchers.Main,
            elapsedRealtimeMs = { testScheduler.currentTime },
        )
        runCurrent()
        harness.viewModel.cancelHostBoundaryJobs()
        advanceUntilIdle()

        assertThat(harness.hostCoreController.trackBoundaryAppliedCalls).isEmpty()
    }
}
