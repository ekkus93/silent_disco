package com.ekkus.silentdisco.app

import android.app.Application
import android.net.Uri
import android.os.SystemClock
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.BuildConfig
import com.ekkus.silentdisco.core.audio.AudioDecodeResult
import com.ekkus.silentdisco.core.audio.AudioFileAccessException
import com.ekkus.silentdisco.core.audio.AudioFileDecoder
import com.ekkus.silentdisco.core.audio.AudioFormatSpec
import com.ekkus.silentdisco.core.audio.DecodedAudioChunk
import com.ekkus.silentdisco.core.audio.ListenerPlaybackScheduler
import com.ekkus.silentdisco.core.audio.AudioTrackPlaybackEngine
import com.ekkus.silentdisco.core.audio.OboeBridge
import com.ekkus.silentdisco.core.audio.PlaybackEngine
import com.ekkus.silentdisco.core.audio.PcmPacketizer
import com.ekkus.silentdisco.core.audio.PlaybackFrame
import com.ekkus.silentdisco.core.audio.PlaybackThresholds
import com.ekkus.silentdisco.core.audio.packetizationStats
import com.ekkus.silentdisco.core.audio.validatePacketBudget
import com.ekkus.silentdisco.core.diagnostics.DiagnosticsStore
import com.ekkus.silentdisco.core.logging.AppLogger
import com.ekkus.silentdisco.core.logging.DiagnosticsMetrics
import com.ekkus.silentdisco.core.model.AppRole
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.JoinApprovalState
import com.ekkus.silentdisco.core.model.JoinRequest
import com.ekkus.silentdisco.core.model.ListenerDiagnosticsSnapshot
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.model.SyncState
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.model.TrustState
import com.ekkus.silentdisco.core.permissions.PermissionCatalogue
import com.ekkus.silentdisco.core.permissions.AppPermission
import com.ekkus.silentdisco.core.permissions.PermissionState
import com.ekkus.silentdisco.core.rust.RustStoredTuningSettings
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.ControlMessage
import com.ekkus.silentdisco.core.protocol.DeviceIdentity
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.protocol.SyncRequestPacket
import com.ekkus.silentdisco.core.protocol.SyncResponsePacket
import com.ekkus.silentdisco.core.sync.HostTimeMapper
import com.ekkus.silentdisco.core.sync.HostTimingService
import com.ekkus.silentdisco.core.sync.ClockSyncEstimator
import com.ekkus.silentdisco.core.sync.ListenerSyncController
import com.ekkus.silentdisco.core.sync.SyncMaintenanceConfig
import com.ekkus.silentdisco.core.transport.BleAdvertisement
import com.ekkus.silentdisco.core.transport.BleDiscoveryService
import com.ekkus.silentdisco.core.transport.BleOperation
import com.ekkus.silentdisco.core.transport.BroadcastDeliverySeverity
import com.ekkus.silentdisco.core.transport.SendAllResult
import com.ekkus.silentdisco.core.transport.classifyBroadcastDelivery
import com.ekkus.silentdisco.core.transport.WifiDirectTransportService
import com.ekkus.silentdisco.platform.persistence.AndroidRustDomainStore
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking

    internal fun MainViewModel.requestListenerSyncProbe(source: String) {
        val session = _uiState.value.selectedSession
        if (session == null) {
            val message = "Join a session before requesting manual resync"
            _uiState.value = _uiState.value.copy(lastError = message)
            diagnosticsStore.updateListener { it.copy(lastError = message) }
            refreshListenerDiagnostics()
            return
        }

        val controller = listenerSyncController ?: createSyncController(SessionId(session.id)).also {
            listenerSyncController = it
        }
        val request = controller.newProbe()
        pendingSyncCorrelationId = request.correlationId

        if (wifiDirectService.snapshot.value.state == TransportConnectionState.CONNECTED) {
            val nextState = nextStateForSyncProbe(_uiState.value.listenerState)
            val nextProgressState = if (nextState == ListenerLifecycleState.SYNCING_CLOCK) {
                ListenerLifecycleState.SYNCING_CLOCK
            } else {
                _uiState.value.connectionProgress.currentState
            }
            _uiState.value = _uiState.value.copy(
                listenerState = nextState,
                connectionProgress = _uiState.value.connectionProgress.copy(
                    currentState = nextProgressState,
                    requested = true,
                    approved = true,
                    connected = true,
                ),
                lastMessage = "$source sync probe sent",
                lastError = null,
            )
            // Clock sync sampling is not yet exposed over the Rust listener
            // transport (control-plane only for this migration block) -- see
            // FfiListenerTransportEvent's doc comment.
            handleSyncFailure("Clock sync is not yet available over the migrated Wi-Fi Direct transport")
            return
        }

        val isDemoSession = BuildConfig.DEBUG && session.id.startsWith("demo-session-")
        if (isDemoSession) {
            applySyncResponse(hostTimingService.createResponse(request))
            _uiState.value = _uiState.value.copy(
                lastMessage = "$source sync applied locally for demo session",
                lastError = null,
            )
            return
        }

        val message = "Manual resync requires an active host connection"
        _uiState.value = _uiState.value.copy(lastError = message)
        diagnosticsStore.updateListener { it.copy(lastError = message) }
        refreshListenerDiagnostics()
    }

    internal fun MainViewModel.applySyncResponse(response: SyncResponsePacket) {
        if (_uiState.value.selectedSession?.id != response.sessionId.value) return
        val expectedCorrelationId = pendingSyncCorrelationId
        if (expectedCorrelationId != null && response.correlationId != expectedCorrelationId) return
        pendingSyncCorrelationId = null
        val controller = listenerSyncController ?: ListenerSyncController(response.sessionId).also {
            listenerSyncController = it
        }
        val syncState = controller.onResponse(response).copy(
            resyncCount = _uiState.value.listenerSyncState.resyncCount + 1,
        )
        val shouldResync = controller.shouldResync(state = syncState)
        logger.i(
            "sync.sample",
            "offset=${"%.2f".format(syncState.offsetMs)} rtt=${"%.2f".format(syncState.rttMs)} jitter=${"%.2f".format(syncState.jitterMs)}",
        )
        if (shouldResync && !_uiState.value.connectionProgress.synced) {
            handleSyncFailure("Unable to establish a stable sync estimate")
            return
        }
        _uiState.value = _uiState.value.copy(
            listenerSyncState = syncState,
            listenerState = if (shouldResync) ListenerLifecycleState.DESYNCED else _uiState.value.listenerState,
            connectionProgress = _uiState.value.connectionProgress.copy(synced = !shouldResync),
        )
        diagnosticsStore.updateListener {
            it.copy(
                hostOffsetMs = syncState.offsetMs,
                rttMs = syncState.rttMs,
                jitterMs = syncState.jitterMs,
                resyncCount = syncState.resyncCount,
                metricsSummary = summarizeMetrics(),
                lastError = if (shouldResync) "Sync drift exceeded threshold" else null,
            )
        }
        metrics.increment("sync_sample")
        metrics.recordTiming("sync_rtt_ms", syncState.rttMs)
        if (shouldResync) {
            metrics.increment("playback_desync")
            logger.w("playback.desync", "Resync threshold exceeded")
        }
        refreshListenerDiagnostics()
        refreshHostDiagnostics()
    }

    internal fun MainViewModel.handleSyncFailure(message: String) {
        logger.w("sync.error", message)
        metrics.increment("sync_establish_failure")
        // Reported into Rust (rather than written locally) so a later Rust
        // snapshot cannot silently revert this back to an earlier state.
        listenerCoreController?.transportFailed(message, retryable = true)
        _uiState.value = _uiState.value.copy(
            listenerPlaybackState = PlaybackState.ERROR,
            connectionProgress = _uiState.value.connectionProgress.copy(
                buffered = false,
                playing = false,
            ),
        )
        diagnosticsStore.updateListener {
            it.copy(
                playbackState = PlaybackState.ERROR,
                lastError = message,
                metricsSummary = summarizeMetrics(),
            )
        }
        refreshListenerDiagnostics()
    }

    internal fun MainViewModel.startPeriodicListenerResync() {
        resyncJob?.cancel()
        resyncJob = viewModelScope.launch {
            while (shouldKeepResyncing()) {
                delay(_uiState.value.tuningSettings.syncCadenceMs)
                if (_uiState.value.canManualResync()) {
                    requestListenerSyncProbe(source = "Periodic listener resync")
                }
                wifiDirectService.recordHeartbeat()
                if (wifiDirectService.snapshot.value.state == TransportConnectionState.DISCONNECTED ||
                    wifiDirectService.snapshot.value.state == TransportConnectionState.FAILED
                ) {
                    handleListenerDisconnect("Transport disconnected during playback")
                    return@launch
                }
            }
        }
    }

    internal fun MainViewModel.createSyncController(sessionId: SessionId): ListenerSyncController {
        val tuning = _uiState.value.tuningSettings
        return ListenerSyncController(
            sessionId = sessionId,
            estimator = ClockSyncEstimator(maxSamples = tuning.syncSampleWindow),
            config = SyncMaintenanceConfig(
                cadenceMs = tuning.syncCadenceMs,
                driftThresholdMs = tuning.syncDriftThresholdMs,
                sampleHistorySize = tuning.syncSampleWindow,
            ),
        )
    }
