package com.ekkus.silentdisco.app

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
            )
        }
        _uiState.value = _uiState.value.copy(hostDiagnostics = diagnosticsStore.hostDiagnostics.value)
    }

    internal fun MainViewModel.refreshListenerDiagnostics() {
        _uiState.value = _uiState.value.copy(listenerDiagnostics = diagnosticsStore.listenerDiagnostics.value)
    }

    internal fun MainViewModel.summarizeMetrics(): String {
        val counters = metrics.snapshotCounters()
        val timings = metrics.snapshotTimings()
        if (counters.isEmpty() && timings.isEmpty()) return "No metrics yet"
        val counterSummary = counters.entries.joinToString(", ") { "${it.key}=${it.value}" }
        val timingSummary = timings.entries.joinToString(", ") { "${it.key}=${"%.1f".format(it.value)}ms" }
        return listOf(counterSummary, timingSummary).filter { it.isNotBlank() }.joinToString(" | ")
    }
