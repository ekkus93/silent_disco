package com.ekkus.silentdisco.app

import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.transport.BleOperation
import com.ekkus.silentdisco.core.transport.BroadcastDeliverySeverity
import com.ekkus.silentdisco.core.transport.SendAllResult
import com.ekkus.silentdisco.core.transport.classifyBroadcastDelivery
import kotlinx.coroutines.launch

    internal fun MainViewModel.observeDiscovery() {
        viewModelScope.launch {
            bleService.discoveredSessions.collect {
                refreshDiscoveredSessions()
            }
        }
        viewModelScope.launch {
            wifiDirectService.snapshot.collect { snapshot ->
                refreshDiscoveredSessions()
                reportRustHostTransportState(snapshot.state)
                handleTransportSnapshot(snapshot)
                completeRustListenerNetworkEstablishment(snapshot)
                completeRustHostAdvertising(snapshot)
            }
        }
    }

    internal fun MainViewModel.handleTransportSnapshot(snapshot: com.ekkus.silentdisco.core.transport.TransportSnapshot) {
        if (snapshot.state != TransportConnectionState.FAILED || snapshot.lastError == null) return
        val errorMessage = snapshot.lastError.message
        val role = classifyTransportSnapshotRole(_uiState.value.hostState, _uiState.value.listenerState)
        when (role) {
            TransportSnapshotRole.HOST_FAILURE -> {
                reportRustHostTransportFailure(errorMessage, snapshot.lastError.retryable)
            }
            TransportSnapshotRole.LISTENER_FAILURE -> {
                handleListenerConnectionFailure(errorMessage)
            }
            TransportSnapshotRole.IGNORE -> Unit
        }
    }

    internal fun MainViewModel.observeBleFailures() {
        viewModelScope.launch {
            bleService.failures.collect { failure ->
                when (failure.operation) {
                    BleOperation.SCAN -> handleBleScanFailure(failure.message)
                    BleOperation.ADVERTISE -> handleBleAdvertiseFailure(failure.message)
                }
            }
        }
    }

    internal fun MainViewModel.handleBleScanFailure(message: String) {
        clearScanState()
        listenerCoreController?.transportFailed(message, retryable = true)
        diagnosticsStore.updateListener { it.copy(lastError = message) }
        refreshListenerDiagnostics()
    }

    internal fun MainViewModel.handleBleAdvertiseFailure(message: String) {
        val hosting = _uiState.value.hostState in setOf(
            HostLifecycleState.CREATING_SESSION,
            HostLifecycleState.WAITING_FOR_LISTENERS,
            HostLifecycleState.READY,
            HostLifecycleState.STREAMING,
        )
        if (!hosting) return
        wifiDirectService.stop()
        reportRustHostTransportFailure(message, retryable = true)
    }

    internal fun MainViewModel.reportHostBroadcastDelivery(
        action: String,
        result: SendAllResult,
        requireAnyPeer: Boolean = true,
    ): Boolean {
        val report = classifyBroadcastDelivery(action, result)
        if (report.severity == BroadcastDeliverySeverity.OK) return true
        val message = report.message ?: "$action delivery issue"
        logger.w("transport.control", message)
        _uiState.value = _uiState.value.copy(lastError = message)
        diagnosticsStore.updateHost {
            it.copy(lastError = message, metricsSummary = summarizeMetrics())
        }
        refreshHostDiagnostics()
        return report.severity == BroadcastDeliverySeverity.ZERO_PEERS && !requireAnyPeer
    }

    internal fun MainViewModel.hostControlDeliveryMessage(
        localActionPastTense: String,
        deliveryAction: String,
        result: SendAllResult,
    ): String? {
        val report = classifyBroadcastDelivery(deliveryAction, result)
        return when (report.severity) {
            BroadcastDeliverySeverity.OK -> null
            BroadcastDeliverySeverity.ZERO_PEERS -> "$localActionPastTense locally; no connected listeners received the command"
            BroadcastDeliverySeverity.PARTIAL_FAILURE -> "$localActionPastTense locally; ${report.message}"
        }
    }

    internal fun MainViewModel.handleHostControlFailure(action: String, error: Throwable) {
        val message = "Failed to broadcast $action to listeners: ${error.message}"
        logger.w("transport.control", message)
        _uiState.value = _uiState.value.copy(lastError = message)
        diagnosticsStore.updateHost {
            it.copy(lastError = message, metricsSummary = summarizeMetrics())
        }
        refreshHostDiagnostics()
    }

    /**
     * Local resource cleanup shared by every listener failure path
     * (transport failure, join rejection, host-initiated disconnect) --
     * lifecycle state itself is reported to the Rust actor by each caller,
     * not written here, so there is exactly one source of truth for it.
     */
    private fun MainViewModel.stopListenerPlaybackForFailure(message: String) {
        clearScanState()
        logger.w("transport.error", message)
        resyncJob?.cancel()
        // Stops the Rust runtime, its pump thread, its ring registration and
        // the native output together. Cancelling the diagnostics job alone
        // leaked all of those on every disconnect and connection failure.
        stopListenerPlayback()
        closeListenerSyncEstimator()
        diagnosticsStore.updateListener {
            it.copy(
                playbackState = PlaybackState.ERROR,
                lastError = message,
                metricsSummary = summarizeMetrics(),
            )
        }
        refreshListenerDiagnostics()
    }

    internal fun MainViewModel.handleListenerConnectionFailure(message: String) {
        stopListenerPlaybackForFailure(message)
        listenerCoreController?.transportFailed(message, retryable = true)
    }

    internal fun MainViewModel.handleListenerDisconnect(message: String) {
        stopListenerPlaybackForFailure(message)
        pendingSyncCorrelationId = null
        // No dedicated Rust event exists yet for a graceful host-initiated
        // disconnect (only Failed); approximating with transportFailed means
        // this surfaces as Error rather than Disconnected in the UI for now.
        listenerCoreController?.transportFailed(message, retryable = true)
    }

    /**
     * Only BLE-advertised sessions are trusted as discovered sessions. Wi-Fi
     * Direct's own peer list is not a signal that a peer is running Silent
     * Disco at all -- it surfaces every nearby Wi-Fi-Direct-capable device
     * (TVs, printers, other phones), and previously got merged in directly,
     * rendering arbitrary nearby devices as joinable "sessions". Wi-Fi
     * Direct is only used for establishment once a BLE-confirmed session has
     * actually been selected (see `establishRustListenerNetwork`).
     */
    internal fun MainViewModel.refreshDiscoveredSessions() {
        val bleSessions = bleService.discoveredSessions.value
            .distinctBy { it.id }
            .sortedBy { it.name }
        val controller = listenerCoreController ?: return
        val known = controller.snapshots.value?.discoveredSessions.orEmpty()
            .map { it.sessionId }
            .toSet()
        val bleSessionIds = bleSessions.map { it.id }.toSet()
        bleSessions.filter { it.id !in known }.forEach { session ->
            controller.submitSessionDiscovered(session.toFfiSessionAdvertisement())
        }
        (known - bleSessionIds).forEach { sessionId -> controller.submitSessionExpired(sessionId) }
    }
