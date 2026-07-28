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

internal fun MainViewModel.scanForSessionsImpl() {
    if (!requirePersistenceReady("scan for sessions")) return
    logger.i("listener.scan", "Scanning for nearby sessions")
    scanJob?.cancel()

    if (!hasListenerTransportPermissions()) {
        val message = "Missing nearby connectivity permissions for discovery"
        wifiDirectService.fail(message, retryable = true)
        _uiState.value = _uiState.value.copy(
            isScanning = false,
            listenerState = ListenerLifecycleState.ERROR,
            lastError = message,
        )
        diagnosticsStore.updateListener { it.copy(lastError = message) }
        refreshListenerDiagnostics()
        return
    }

    _uiState.value = _uiState.value.copy(
        isScanning = true,
        listenerState = ListenerLifecycleState.SCANNING,
        connectionProgress = _uiState.value.connectionProgress.copy(
            currentState = ListenerLifecycleState.SCANNING,
        ),
        lastError = null,
        lastMessage = "Scanning for nearby sessions…",
    )

    scanJob = viewModelScope.launch {
        val bleStart = bleService.startScanning()
        if (!bleStart.started) {
            val message = bleStart.message ?: "BLE scan could not start"
            wifiDirectService.fail(message, retryable = true)
            _uiState.value = _uiState.value.copy(
                isScanning = false,
                listenerState = ListenerLifecycleState.ERROR,
                lastError = message,
            )
            diagnosticsStore.updateListener { it.copy(lastError = message) }
            refreshListenerDiagnostics()
            return@launch
        }

        wifiDirectService.discoverPeers()
        val scanWindowMs = _uiState.value.tuningSettings.normalized().scanWindowMs
        delay(scanWindowMs)
        refreshDiscoveredSessions()

        val discovered = _uiState.value.discoveredSessions
        _uiState.value = _uiState.value.copy(
            isScanning = false,
            listenerState = if (_uiState.value.selectedSession == null) {
                ListenerLifecycleState.IDLE
            } else {
                ListenerLifecycleState.SESSION_SELECTED
            },
            discoveredSessions = discovered,
            connectionProgress = _uiState.value.connectionProgress.copy(
                currentState = if (discovered.isEmpty()) {
                    ListenerLifecycleState.IDLE
                } else {
                    ListenerLifecycleState.SESSION_SELECTED
                },
                discovered = discovered.isNotEmpty(),
            ),
            lastMessage = if (discovered.isEmpty()) "No nearby sessions found" else "Found ${discovered.size} session(s)",
            lastError = null,
        )
        diagnosticsStore.updateListener { it.copy(lastError = null) }
        refreshListenerDiagnostics()
    }
}

internal fun MainViewModel.requestJoinImpl() {
    if (!requirePersistenceReady("join a session")) return
    val session = _uiState.value.selectedSession ?: run {
        _uiState.value = _uiState.value.copy(lastError = "Select a session before joining")
        return
    }
    if (_uiState.value.discoveredSessions.none { it.id == session.id }) {
        _uiState.value = _uiState.value.copy(lastError = "Selected session disappeared before join")
        diagnosticsStore.updateListener { it.copy(lastError = "Selected session disappeared before join") }
        refreshListenerDiagnostics()
        return
    }
    if (session.inviteCodeRequired && _uiState.value.connectionProgress.inviteCode.isBlank()) {
        _uiState.value = _uiState.value.copy(lastError = "Invite code required")
        return
    }
    val request = ControlMessage.JoinRequest(
        version = 1,
        sessionId = SessionId(session.id),
        device = DeviceIdentity(localListenerDeviceId, "This Android Listener"),
        inviteCode = _uiState.value.connectionProgress.inviteCode.ifBlank { null },
    )
    logger.i("listener.join", "Join request created for ${request.sessionId.value}")
    _uiState.value = _uiState.value.copy(
        listenerState = ListenerLifecycleState.CONNECTING,
        connectionProgress = _uiState.value.connectionProgress.copy(
            currentState = ListenerLifecycleState.CONNECTING,
            requested = true,
        ),
        lastMessage = "Connecting to host",
        lastError = null,
    )
    pendingJoinRequestMessage = request
    val shouldSimulate = BuildConfig.DEBUG && session.id.startsWith("demo-session-")
    if (shouldSimulate) {
        val shouldReject = session.inviteCodeRequired && request.inviteCode != "1234"
        simulateApprovalAndPlayback(session.id, shouldReject)
        return
    }
    wifiDirectService.connectToSession(session)
}

internal fun MainViewModel.startTransportListenerPlayback(sessionId: SessionId, streamId: StreamId, format: AudioFormatSpec = AudioFormatSpec()) {
    val mapper = HostTimeMapper(
        offsetMs = _uiState.value.listenerSyncState.offsetMs,
        skewPpm = _uiState.value.listenerSyncState.skewPpm,
    )
    listenerScheduler = ListenerPlaybackScheduler(
        mapper = mapper,
        thresholds = currentPlaybackThresholds(),
        expectedSessionId = sessionId,
        expectedStreamId = streamId,
    )
    pendingTransportPackets
        .filter { it.sessionId == sessionId && it.streamId == streamId }
        .forEach { packet -> listenerScheduler?.let { recordIncomingPacket(it, packet) } }
    pendingTransportPackets.clear()
    runCatching { playbackEngine.start(format) }.onFailure { error ->
        handleListenerPlaybackEngineFailure(error)
        return
    }
    playbackJob?.cancel()
    playbackJob = viewModelScope.launch {
        var started = false
        var lastUnderrunCount = 0
        while (_uiState.value.listenerState != ListenerLifecycleState.DISCONNECTED) {
            if (wifiDirectService.snapshot.value.state == TransportConnectionState.DISCONNECTED ||
                wifiDirectService.snapshot.value.state == TransportConnectionState.FAILED
            ) {
                handleListenerDisconnect("Transport disconnected during playback")
                return@launch
            }
            val scheduler = listenerScheduler ?: return@launch
            if (!started) {
                if (!scheduler.canStart()) {
                    delay(10)
                    continue
                }
                started = true
                _uiState.value = _uiState.value.copy(
                    listenerState = ListenerLifecycleState.PLAYING,
                    listenerPlaybackState = PlaybackState.PLAYING,
                    connectionProgress = _uiState.value.connectionProgress.copy(
                        currentState = ListenerLifecycleState.PLAYING,
                        connected = true,
                        approved = true,
                        synced = true,
                        buffered = true,
                        playing = true,
                    ),
                )
            }
            val frame = scheduler.poll()
            if (frame == null) {
                delay(10)
                continue
            }
            runCatching { playbackEngine.write(frame) }.onFailure { error ->
                handleListenerPlaybackEngineFailure(error)
                return@launch
            }
            val telemetry = scheduler.snapshot()
            if (frame.concealed) {
                logger.w("packet.receive.anomaly", "Inserted concealment for seq=${frame.packet.sequenceNumber}")
            }
            if (telemetry.underrunCount > lastUnderrunCount) {
                logger.w("playback.underrun", "Underrun count=${telemetry.underrunCount}")
                lastUnderrunCount = telemetry.underrunCount
            }
            diagnosticsStore.updateListener {
                it.copy(
                    playbackState = if (telemetry.underrunCount > 0) PlaybackState.UNDERRUN else PlaybackState.PLAYING,
                    playbackPositionMs = playbackEngine.playbackPositionMs(frame),
                    bufferDepthMs = telemetry.bufferDepthMs,
                    packetLossCount = telemetry.packetLossCount,
                    lateDropCount = telemetry.lateDropCount,
                    underrunCount = telemetry.underrunCount,
                    invalidPacketCount = telemetry.invalidPacketCount,
                    concealedPacketCount = telemetry.concealedPacketCount,
                    lastPacketSequence = telemetry.lastPlayedSequence,
                    metricsSummary = summarizeMetrics(),
                )
            }
            if (telemetry.shouldResync) {
                _uiState.value = _uiState.value.copy(listenerState = ListenerLifecycleState.DESYNCED)
            }
            refreshListenerDiagnostics()
            delay(20)
        }
    }
}
