package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.transport.MdnsSessionAdvertisement
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Before
import org.junit.Test

/**
 * Block 52.3: Android kept reconciling mDNS results into the core after
 * discovery stopped. Stopping mDNS clears its session list, so joining (which
 * stops discovery) made the reconciliation expire the very session the
 * listener had just joined, and resubmitted results the core rejected as
 * "discovery result arrived while discovery was inactive".
 */
@OptIn(ExperimentalCoroutinesApi::class)
class MainViewModelDiscoveryReconcileTest {
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
    fun nothingIsReconciledWhileTheCoreIsNotDiscovering() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.ensureRustListenerCore()
        harness.listenerCoreController.publishSnapshot(
            listenerCoreSnapshot(discoveryActive = false, known = listOf("joined"), selected = "joined"),
        )
        advanceUntilIdle()

        // Stopping discovery empties the list...
        harness.mdnsService.setSessions(emptyList())
        advanceUntilIdle()
        // ...and a late result still arrives afterwards.
        harness.mdnsService.setSessions(listOf(mdns("late")))
        advanceUntilIdle()

        assertThat(harness.listenerCoreController.submitSessionExpiredCalls).isEmpty()
        assertThat(harness.listenerCoreController.submitSessionDiscoveredCalls).isEmpty()
    }

    @Test
    fun resultsAreReconciledWhileTheCoreIsDiscovering() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.ensureRustListenerCore()
        harness.listenerCoreController.publishSnapshot(
            listenerCoreSnapshot(discoveryActive = true, known = listOf("gone"), selected = null),
        )
        advanceUntilIdle()

        harness.mdnsService.setSessions(listOf(mdns("fresh")))
        advanceUntilIdle()

        assertThat(harness.listenerCoreController.submitSessionDiscoveredCalls.map { it.sessionId })
            .contains("fresh")
        assertThat(harness.listenerCoreController.submitSessionExpiredCalls).contains("gone")
    }

    /**
     * Gating on the core's discovery state must not strand results that were
     * already found when discovery becomes active.
     */
    @Test
    fun resultsAlreadyFoundAreSubmittedWhenDiscoveryStarts() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.ensureRustListenerCore()
        harness.listenerCoreController.publishSnapshot(
            listenerCoreSnapshot(discoveryActive = false, known = emptyList(), selected = null),
        )
        harness.mdnsService.setSessions(listOf(mdns("waiting")))
        advanceUntilIdle()
        assertThat(harness.listenerCoreController.submitSessionDiscoveredCalls).isEmpty()

        harness.listenerCoreController.publishSnapshot(
            listenerCoreSnapshot(discoveryActive = true, known = emptyList(), selected = null),
        )
        advanceUntilIdle()

        assertThat(harness.listenerCoreController.submitSessionDiscoveredCalls.map { it.sessionId })
            .containsExactly("waiting")
    }

    /**
     * Block 52.5: CONNECTING precedes the host's reply, so it must not set the
     * approved flag -- that ticked "Waiting for host approval" before the host
     * had approved anything.
     */
    @Test
    fun aConnectingListenerIsNotReportedAsApproved() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.ensureRustListenerCore()
        harness.listenerCoreController.publishSnapshot(
            listenerCoreSnapshot(discoveryActive = false, known = listOf("s"), selected = "s"),
        )
        advanceUntilIdle()

        val progress = harness.viewModel.uiState.value.connectionProgress
        assertThat(progress.connected).isTrue()
        assertThat(progress.approved).isFalse()
        assertThat(harness.viewModel.uiState.value.joinUiStep())
            .isEqualTo(JoinUiStep.REQUESTING_ACCESS)
    }

    private fun mdns(id: String) = MdnsSessionAdvertisement(
        sessionId = id,
        hostDeviceId = "desktop-host",
        sessionName = "Session $id",
        approvalMode = ApprovalMode.MANUAL,
        protocolVersion = 2,
        address = "192.168.1.50",
        controlPort = 41_000,
        syncPort = 41_001,
        audioPort = 41_002,
    )
}
