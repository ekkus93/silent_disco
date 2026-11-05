package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.BridgeQueueCounters
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.model.TransportConnectionState

    internal fun MainViewModel.refreshHostDiagnostics(
        streamState: PlaybackState = _uiState.value.hostPlaybackState,
        sessionId: String = _uiState.value.hostDiagnostics.sessionId,
    ) {
        diagnosticsStore.updateHost {
            it.copy(
                sessionId = sessionId,
                listenerCount = _uiState.value.approvedListeners.size + _uiState.value.pendingJoinRequests.size,
                pendingJoinCount = _uiState.value.pendingJoinRequests.size,
                connectedListenerCount = _uiState.value.approvedListeners.count {
                    it.connectionState == TransportConnectionState.CONNECTING || it.connectionState == TransportConnectionState.CONNECTED
                },
                desyncedListenerCount = _uiState.value.approvedListeners.count {
                    it.listenerState == ListenerLifecycleState.DESYNCED || it.syncQuality == SyncQualityBadge.POOR
                } + if (_uiState.value.listenerState == ListenerLifecycleState.DESYNCED) 1 else 0,
                streamState = streamState,
                lastContactElapsedMs = wifiDirectService.snapshot.value.lastContactElapsedMs,
                metricsSummary = summarizeMetrics(),
                packetBudgetSummary = it.packetBudgetSummary,
                lastError = _uiState.value.lastError,
                bridgeQueues = hostBridgeQueueCounters(),
            )
        }
        _uiState.value = _uiState.value.copy(hostDiagnostics = diagnosticsStore.hostDiagnostics.value)
    }

    internal fun MainViewModel.refreshListenerDiagnostics() {
        diagnosticsStore.updateListener { it.copy(bridgeQueues = listenerBridgeQueueCounters()) }
        _uiState.value = _uiState.value.copy(listenerDiagnostics = diagnosticsStore.listenerDiagnostics.value)
    }

    /** Host-side bridge pressure: transport telemetry drops and core queue drops/timeouts. */
    internal fun MainViewModel.hostBridgeQueueCounters(): BridgeQueueCounters = BridgeQueueCounters(
        droppedBestEffortEvents = hostTransportController.droppedDiagnosticEvents,
        droppedDiagnosticNotifications = hostCoreController?.droppedDiagnosticNotifications ?: 0L,
        criticalNotificationTimeouts = hostCoreController?.criticalNotificationTimeouts ?: 0L,
    )

    /** Listener-side bridge pressure: PCM notification drops and core queue drops/timeouts. */
    internal fun MainViewModel.listenerBridgeQueueCounters(): BridgeQueueCounters = BridgeQueueCounters(
        droppedBestEffortEvents = listenerTransportController.droppedAudioEvents,
        droppedDiagnosticNotifications = listenerCoreController?.droppedDiagnosticNotifications ?: 0L,
        criticalNotificationTimeouts = listenerCoreController?.criticalNotificationTimeouts ?: 0L,
    )

    /**
     * The counters change when a collector stalls, not when a diagnostics
     * refresh happens to run, so read them again at the moment a report is built.
     */
    internal fun MainViewModel.uiStateForSupportReport(): AppUiState {
        refreshHostDiagnostics()
        refreshListenerDiagnostics()
        return _uiState.value
    }

    internal fun MainViewModel.summarizeMetrics(): String {
        val counters = metrics.snapshotCounters()
        val timings = metrics.snapshotTimings()
        if (counters.isEmpty() && timings.isEmpty()) return "No metrics yet"
        val counterSummary = counters.entries.joinToString(", ") { "${it.key}=${it.value}" }
        val timingSummary = timings.entries.joinToString(", ") { "${it.key}=${"%.1f".format(it.value)}ms" }
        return listOf(counterSummary, timingSummary).filter { it.isNotBlank() }.joinToString(" | ")
    }
