package com.ekkus.silentdisco.app

import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStore
import com.ekkus.silentdisco.core.rust.RustDatabaseException
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import com.ekkus.silentdisco.core.uniffi.FfiPlaybackState
import com.google.common.truth.Truth.assertThat
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.mockito.Mockito.inOrder
import org.mockito.Mockito.mock

/**
 * Characterization tests written before splitting `MainViewModel` into
 * controllers (God-file refactor TODO §2, spec Appendix A.5 gaps). They pin
 * today's behaviour of the paths no other test covers, so each controller
 * extraction can prove it kept that behaviour.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class MainViewModelRefactorSafetyNetTest {
    private val dispatcher = StandardTestDispatcher()

    @Before
    fun setUpMain() {
        Dispatchers.setMain(dispatcher)
    }

    @After
    fun resetMain() {
        Dispatchers.resetMain()
    }

    /** Some paths hop to the real `Dispatchers.IO`; wait for them, bounded. */
    private fun TestScope.awaitCondition(condition: () -> Boolean) {
        repeat(200) {
            advanceUntilIdle()
            if (condition()) return
            Thread.sleep(10)
        }
        check(condition()) { "Condition was not met within the bounded wait" }
    }

    /** Clears the view-model the way the framework does, through its store. */
    private fun clear(viewModel: MainViewModel) {
        val store = ViewModelStore()
        val factory = object : ViewModelProvider.Factory {
            @Suppress("UNCHECKED_CAST")
            override fun <T : ViewModel> create(modelClass: Class<T>): T = viewModel as T
        }
        ViewModelProvider(store, factory)[MainViewModel::class.java]
        store.clear()
    }

    @Test
    fun clearingTheViewModelStopsEveryServiceAndClosesEveryController() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        val viewModel = harness.viewModel
        advanceUntilIdle()
        viewModel.ensureRustHostCore()
        advanceUntilIdle()

        clear(viewModel)

        assertThat(harness.bleService.stopCallCount).isEqualTo(1)
        assertThat(harness.wifiDirectService.stopCallCount).isEqualTo(1)
        assertThat(harness.mdnsService.stopCallCount).isEqualTo(1)
        assertThat(harness.hostCoreController.closeCallCount).isEqualTo(1)
        assertThat(harness.listenerTransport.closeCallCount).isEqualTo(1)
        assertThat(harness.domainStore.closeCallCount).isEqualTo(1)
        assertThat(viewModel.listenerPlaybackController.executor.isShutdown).isTrue()
    }

    @Test
    fun aRecoverableStorageFailureCanBeRetriedAndAFatalOneCannot() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.domainStore.initializeFailure = RustDatabaseException(-105, "open database")
        // Construction already started initialization; let it fail.
        advanceUntilIdle()
        val viewModel = harness.viewModel
        // The constructor ran before the failure was set, so retry once to
        // observe the recoverable failure itself.
        viewModel.initializeDomainPersistence()
        advanceUntilIdle()
        assertThat(viewModel.uiState.value.storageState)
            .isEqualTo(StorageInitializationState.RECOVERABLE_FAILURE)

        harness.domainStore.initializeFailure = null
        val callsBefore = harness.domainStore.initializeCallCount
        viewModel.retryDomainPersistence()
        advanceUntilIdle()
        assertThat(harness.domainStore.initializeCallCount).isEqualTo(callsBefore + 1)
        assertThat(viewModel.uiState.value.storageState).isEqualTo(StorageInitializationState.READY)

        // A fatal failure is not offered for retry.
        harness.domainStore.initializeFailure = RustDatabaseException(-103, "open database")
        viewModel.initializeDomainPersistence()
        advanceUntilIdle()
        assertThat(viewModel.uiState.value.storageState)
            .isEqualTo(StorageInitializationState.FATAL_FAILURE)
        val callsAtFatal = harness.domainStore.initializeCallCount
        viewModel.retryDomainPersistence()
        advanceUntilIdle()
        assertThat(harness.domainStore.initializeCallCount).isEqualTo(callsAtFatal)
    }

    @Test
    fun stoppingTheHostMonitorStopsClosesOutputAndClosesOnceInOrder() = runTest(dispatcher) {
        var outputCloses = 0
        val harness = newTestMainViewModelHarness(closeHostMonitorOutput = { outputCloses += 1 })
        val viewModel = harness.viewModel
        advanceUntilIdle()

        // Idle: stopping does nothing, not even closing the output.
        viewModel.stopHostMonitorPlayback()
        assertThat(outputCloses).isEqualTo(0)

        val runtime = mock(FfiListenerPlaybackHandle::class.java)
        viewModel.swapHostMonitorPlayback(runtime)
        viewModel.stopHostMonitorPlayback()
        val order = inOrder(runtime)
        order.verify(runtime).stop()
        order.verify(runtime).close()
        assertThat(outputCloses).isEqualTo(1)
        assertThat(viewModel.hostMonitorPlayback).isNull()

        // Idempotent.
        viewModel.stopHostMonitorPlayback()
        assertThat(outputCloses).isEqualTo(1)
    }

    /** Spec A.3 item 1: stopping the host also cancels the listener-role jobs. */
    @Test
    fun stoppingHostPlaybackCancelsTheStreamAndTheListenerRoleJobs() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        val viewModel = harness.viewModel
        advanceUntilIdle()
        harness.hostCoreController.transitionSnapshot =
            listenerCoreSnapshot(discoveryActive = false, known = emptyList(), selected = null)
        viewModel.ensureRustHostCore()
        val stream = Job()
        val playback = Job()
        val resync = Job()
        viewModel.hostStreamJob = stream
        viewModel.playbackJob = playback
        viewModel.resyncJob = resync

        viewModel.stopHostPlayback()
        awaitCondition { stream.isCancelled && playback.isCancelled && resync.isCancelled }

        assertThat(harness.hostCoreController.transitionPlaybackStateCalls)
            .containsExactly(FfiPlaybackState.STOPPED)
    }

    /** Spec A.3 item 3: ending the session releases the host's staged audio. */
    @Test
    fun endingTheHostSessionDeletesEveryStagedSourceFile() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        val viewModel = harness.viewModel
        advanceUntilIdle()
        viewModel.ensureRustHostCore()
        val staged = File.createTempFile("staged-source", ".wav")
        viewModel.stagedAudioFilesBySourceId["source-1"] = staged

        viewModel.endSession()
        awaitCondition { viewModel.stagedAudioFilesBySourceId.isEmpty() }

        assertThat(harness.hostCoreController.endHostSessionCallCount).isEqualTo(1)
        assertThat(staged.exists()).isFalse()
    }
}
