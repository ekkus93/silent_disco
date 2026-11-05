package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.rust.HostTransportController
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportHandle
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
import org.mockito.Mockito

/**
 * Bridge queue pressure must be visible in diagnostics: a stalled collector
 * drops telemetry and times out critical effects, and none of that may look
 * like a healthy but quiet session in the support report.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class BridgeQueueDiagnosticsTest {
    private val dispatcher = StandardTestDispatcher()

    @Before
    fun setUpMain() {
        Dispatchers.setMain(dispatcher)
    }

    @After
    fun resetMain() {
        Dispatchers.resetMain()
    }

    @Test
    fun hostDiagnosticsReportRealTransportDropsAndCoreQueueCounters() = runTest(dispatcher) {
        // A native handle that streams best-effort heartbeats while nothing
        // collects them fills the reserved telemetry slots, then drops.
        val native = Mockito.mock(FfiHostTransportHandle::class.java)
        Mockito.doAnswer {
            Thread.sleep(1)
            FfiHostTransportEvent.Heartbeat("listener-1")
        }.`when`(native).pollEvent(500uL)
        val transport = HostTransportController(
            callDispatcher = Dispatchers.IO,
            bindHandle = { _, _, _, _, _ -> native },
        )
        val harness = newTestMainViewModelHarness(hostTransportController = transport)
        try {
            transport.bind(backgroundScope, "127.0.0.1", 1, 2, 3, "session-1")
            repeat(400) {
                if (transport.droppedDiagnosticEvents > 0) return@repeat
                Thread.sleep(5)
            }
            assertThat(transport.droppedDiagnosticEvents).isGreaterThan(0L)
            harness.viewModel.ensureRustHostCore()
            advanceUntilIdle()
            harness.hostCoreController.droppedDiagnosticNotifications = 3L
            harness.hostCoreController.criticalNotificationTimeouts = 2L

            harness.viewModel.refreshHostDiagnostics()

            val bridge = harness.viewModel.uiState.value.hostDiagnostics.bridgeQueues
            assertThat(bridge.droppedBestEffortEvents).isGreaterThan(0L)
            assertThat(bridge.droppedDiagnosticNotifications).isEqualTo(3L)
            assertThat(bridge.criticalNotificationTimeouts).isEqualTo(2L)
        } finally {
            transport.close()
        }
    }

    @Test
    fun listenerDiagnosticsReportPcmDropsAndCoreQueueCounters() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.listenerCoreController = harness.listenerCoreController
        harness.listenerTransport.droppedAudioEvents = 7L
        harness.listenerCoreController.droppedDiagnosticNotifications = 5L
        harness.listenerCoreController.criticalNotificationTimeouts = 1L

        harness.viewModel.refreshListenerDiagnostics()

        val bridge = harness.viewModel.uiState.value.listenerDiagnostics.bridgeQueues
        assertThat(bridge.droppedBestEffortEvents).isEqualTo(7L)
        assertThat(bridge.droppedDiagnosticNotifications).isEqualTo(5L)
        assertThat(bridge.criticalNotificationTimeouts).isEqualTo(1L)
    }

    @Test
    fun supportReportReadsCountersAtReportTimeNotAtTheLastRefresh() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.listenerCoreController = harness.listenerCoreController
        harness.viewModel.refreshListenerDiagnostics()
        assertThat(harness.viewModel.uiState.value.listenerDiagnostics.bridgeQueues.droppedBestEffortEvents)
            .isEqualTo(0L)

        // A collector stalls after the last refresh.
        harness.listenerTransport.droppedAudioEvents = 11L
        harness.listenerCoreController.criticalNotificationTimeouts = 4L

        val report = harness.viewModel.uiStateForSupportReport()
            .buildSupportReport(appVersion = "test", generatedAt = "now")

        assertThat(report).contains(
            "Listener bridge queues: telemetry/PCM drops=11, diagnostic drops=0, critical timeouts=4",
        )
        assertThat(report).contains(
            "Host bridge queues: telemetry/PCM drops=0, diagnostic drops=0, critical timeouts=0",
        )
    }
}
