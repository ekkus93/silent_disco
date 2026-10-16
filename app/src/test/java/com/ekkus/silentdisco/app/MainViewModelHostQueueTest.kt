package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.rust.FakeHostCoreController
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Before
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class MainViewModelHostQueueTest {
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
    fun liveQueueReorderAndRemovalSubmitTypedRustCommands() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel._uiState.value = AppUiState(
            hostPlaybackState = PlaybackState.PLAYING,
            hostQueue = listOf(
                HostQueueItem("current", "source-1", "Current", null, null),
                HostQueueItem("next", "source-2", "Next", null, null),
                HostQueueItem("later", "source-3", "Later", null, null),
            ),
        )

        harness.viewModel.reorderLiveQueue(listOf("current", "later", "next"))
        harness.viewModel.removeLiveQueueEntry("next")
        advanceUntilIdle()

        assertThat(harness.hostCoreController.reorderHostQueueCalls)
            .containsExactly(listOf("current", "later", "next"))
        assertThat(harness.hostCoreController.removeHostQueueEntryCalls)
            .containsExactly("next")
        assertThat(harness.viewModel._uiState.value.lastError).isNull()
    }

    @Test
    fun staleQueueCommandFailureRemainsVisibleToTheHost() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.hostCoreController.queueCommandFailure = IllegalStateException("queue changed")

        harness.viewModel.reorderLiveQueue(listOf("current", "next"))
        advanceUntilIdle()

        assertThat(harness.hostCoreController.reorderHostQueueCalls).isEmpty()
        assertThat(harness.viewModel._uiState.value.lastError).isEqualTo("queue changed")
    }

    @Test
    fun hostPlaybackFailureUsesRetryableRustFailureEvent() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.hostCoreController = harness.hostCoreController

        harness.viewModel.reportRustHostPlaybackFailure("Second song could not be decoded")

        assertThat(harness.hostCoreController.playbackFailureCalls)
            .containsExactly("Second song could not be decoded")
        assertThat(harness.hostCoreController.playbackStateChangedCalls).isEmpty()
    }

    @Test
    fun stoppingPlaybackCancelsPendingBoundaryApplicationsBeforeSubmittingStop() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        val pendingBoundary = Job()
        harness.viewModel.hostBoundaryJobs[960L] = pendingBoundary
        harness.viewModel._uiState.value = AppUiState(hostPlaybackState = PlaybackState.PLAYING)

        harness.viewModel.stopHostPlaybackImpl()

        assertThat(pendingBoundary.isCancelled).isTrue()
        assertThat(harness.viewModel.hostBoundaryJobs).isEmpty()
        assertThat(harness.hostCoreController.transitionPlaybackStateCalls).isEmpty()
    }

}
