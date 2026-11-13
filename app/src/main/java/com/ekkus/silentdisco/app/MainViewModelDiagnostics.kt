package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.app.controller.HostDiagnosticsInput
import com.ekkus.silentdisco.core.model.BridgeQueueCounters
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.model.TransportConnectionState

    internal fun MainViewModel.refreshHostDiagnostics(
        streamState: PlaybackState = _uiState.value.hostPlaybackState,
        sessionId: String = _uiState.value.hostDiagnostics.sessionId,
    ) {
        val state = _uiState.value
        val snapshot = diagnostics.refreshHost(
            HostDiagnosticsInput(
                sessionId = sessionId,
                listenerCount = state.approvedListeners.size + state.pendingJoinRequests.size,
                pendingJoinCount = state.pendingJoinRequests.size,
                connectedListenerCount = state.approvedListeners.count {
                    it.connectionState == TransportConnectionState.CONNECTING || it.connectionState == TransportConnectionState.CONNECTED
                },
                desyncedListenerCount = state.approvedListeners.count {
                    it.listenerState == ListenerLifecycleState.DESYNCED || it.syncQuality == SyncQualityBadge.POOR
                } + if (state.listenerState == ListenerLifecycleState.DESYNCED) 1 else 0,
                streamState = streamState,
                lastContactElapsedMs = wifiDirectService.snapshot.value.lastContactElapsedMs,
                lastError = state.lastError,
            ),
        )
        _uiState.value = _uiState.value.copy(hostDiagnostics = snapshot)
    }

    internal fun MainViewModel.refreshListenerDiagnostics() {
        _uiState.value = _uiState.value.copy(listenerDiagnostics = diagnostics.refreshListener())
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

    internal fun MainViewModel.summarizeMetrics(): String = diagnostics.summarizeMetrics()
