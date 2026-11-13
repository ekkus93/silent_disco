package com.ekkus.silentdisco.app.controller

import com.ekkus.silentdisco.core.model.BridgeQueueCounters
import com.ekkus.silentdisco.core.model.PlaybackState
import com.google.common.truth.Truth.assertThat
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Test

class DiagnosticsControllerTest {
    private val parent = CoroutineScope(SupervisorJob())

    @After
    fun cancelParent() = parent.cancel()

    private var hostCounters = BridgeQueueCounters()
    private var listenerCounters = BridgeQueueCounters()

    private fun controller(captureEnabled: Boolean = true) = DiagnosticsController(
        parentScope = parent,
        hostBridgeCounters = { hostCounters },
        listenerBridgeCounters = { listenerCounters },
        captureEnabled = captureEnabled,
    )

    private val input = HostDiagnosticsInput(
        sessionId = "session-1",
        listenerCount = 3,
        pendingJoinCount = 1,
        connectedListenerCount = 2,
        desyncedListenerCount = 1,
        streamState = PlaybackState.PLAYING,
        lastContactElapsedMs = 42L,
        lastError = "late packets",
    )

    @Test
    fun aHostRefreshCombinesTheInputMetricsAndBridgePressure() {
        val diagnostics = controller()
        diagnostics.updateHost { it.copy(packetBudgetSummary = "budget ok") }
        diagnostics.metrics.increment("packets_sent")
        hostCounters = BridgeQueueCounters(droppedBestEffortEvents = 7)

        val snapshot = diagnostics.refreshHost(input)

        assertThat(snapshot.sessionId).isEqualTo("session-1")
        assertThat(snapshot.listenerCount).isEqualTo(3)
        assertThat(snapshot.pendingJoinCount).isEqualTo(1)
        assertThat(snapshot.connectedListenerCount).isEqualTo(2)
        assertThat(snapshot.desyncedListenerCount).isEqualTo(1)
        assertThat(snapshot.streamState).isEqualTo(PlaybackState.PLAYING)
        assertThat(snapshot.lastContactElapsedMs).isEqualTo(42L)
        assertThat(snapshot.lastError).isEqualTo("late packets")
        assertThat(snapshot.metricsSummary).contains("packets_sent=1")
        assertThat(snapshot.packetBudgetSummary).isEqualTo("budget ok")
        assertThat(snapshot.bridgeQueues.droppedBestEffortEvents).isEqualTo(7)
        assertThat(diagnostics.hostDiagnostics.value).isEqualTo(snapshot)
    }

    @Test
    fun aListenerRefreshRereadsBridgePressureAndKeepsOtherFields() {
        val diagnostics = controller()
        diagnostics.updateListener { it.copy(lastError = "sync stalled") }
        listenerCounters = BridgeQueueCounters(criticalNotificationTimeouts = 2)

        val snapshot = diagnostics.refreshListener()

        assertThat(snapshot.bridgeQueues.criticalNotificationTimeouts).isEqualTo(2)
        assertThat(snapshot.lastError).isEqualTo("sync stalled")
    }

    @Test
    fun metricsSummarySaysWhenThereIsNothingYet() {
        assertThat(controller().summarizeMetrics()).isEqualTo("No metrics yet")
    }

    @Test
    fun captureIsIgnoredOutsideDebugBuilds() {
        val diagnostics = controller(captureEnabled = false)
        diagnostics.startCapture(playback = null, directory = File("unused"))
        assertThat(diagnostics.captureStatus.value).isEqualTo("Capture off")
    }

    @Test
    fun captureNeedsRunningPlaybackAndADirectory() {
        val diagnostics = controller()
        diagnostics.startCapture(playback = null, directory = File("unused"))
        assertThat(diagnostics.captureStatus.value)
            .isEqualTo("Connect and start listener playback before recording")
    }

    @Test
    fun stoppingOrFinishingWithNoCaptureDoesNothing() {
        val diagnostics = controller()
        diagnostics.stopCapture()
        diagnostics.finishCaptureForTeardown()
        diagnostics.close()
        assertThat(diagnostics.captureStatus.value).isEqualTo("Capture off")
    }
}
