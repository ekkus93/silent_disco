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

    internal fun MainViewModel.observeTransport() {
        viewModelScope.launch {
            wifiDirectService.controlMessages.collect { message -> handleControlMessage(message) }
        }
        viewModelScope.launch {
            wifiDirectService.syncRequests.collect { request ->
                if (request.sessionId != currentSessionId) return@collect
                viewModelScope.launch {
                    runCatching {
                        wifiDirectService.broadcastSyncResponse(hostTimingService.createResponse(request))
                    }.onSuccess { result ->
                        reportHostBroadcastDelivery("broadcast sync response", result, requireAnyPeer = true)
                    }.onFailure { error ->
                        handleHostControlFailure("broadcast sync response", error)
                    }
                }
            }
        }
        viewModelScope.launch {
            wifiDirectService.syncResponses.collect { response -> applySyncResponse(response) }
        }
        viewModelScope.launch {
            wifiDirectService.audioPackets.collect { packet -> handleIncomingAudioPacket(packet) }
        }
    }

    internal fun MainViewModel.observeDiscovery() {
        viewModelScope.launch {
            bleService.discoveredSessions.collect {
                refreshDiscoveredSessions()
            }
        }
        viewModelScope.launch {
            wifiDirectService.snapshot.collect { snapshot ->
                refreshDiscoveredSessions()
                handleTransportSnapshot(snapshot)
                if (pendingJoinRequestMessage != null && snapshot.state == TransportConnectionState.CONNECTED) {
                    sendPendingJoinRequest()
                }
                if (_uiState.value.listenerState == ListenerLifecycleState.SCANNING &&
                    _uiState.value.discoveredSessions.isEmpty() &&
                    snapshot.state == TransportConnectionState.DISCOVERING
                ) {
                    _uiState.value = _uiState.value.copy(lastMessage = "Scanning for nearby sessions...")
                }
                snapshot.lastError?.let { error ->
                    if (_uiState.value.listenerState == ListenerLifecycleState.CONNECTING ||
                        _uiState.value.listenerState == ListenerLifecycleState.SCANNING
                    ) {
                        _uiState.value = _uiState.value.copy(lastError = error.message)
                    }
                }
            }
        }
    }

    internal fun MainViewModel.handleTransportSnapshot(snapshot: com.ekkus.silentdisco.core.transport.TransportSnapshot) {
        if (snapshot.state != TransportConnectionState.FAILED || snapshot.lastError == null) return
        val errorMessage = snapshot.lastError.message
        val role = classifyTransportSnapshotRole(_uiState.value.hostState, _uiState.value.listenerState)
        when (role) {
            TransportSnapshotRole.HOST_FAILURE -> {
                _uiState.value = _uiState.value.copy(
                    hostState = HostLifecycleState.ERROR,
                    hostPlaybackState = if (_uiState.value.hostPlaybackState == PlaybackState.PLAYING) {
                        PlaybackState.ERROR
                    } else {
                        _uiState.value.hostPlaybackState
                    },
                    lastError = errorMessage,
                )
                diagnosticsStore.updateHost {
                    it.copy(lastError = errorMessage, metricsSummary = summarizeMetrics())
                }
                refreshHostDiagnostics()
            }
            TransportSnapshotRole.LISTENER_FAILURE -> {
                pendingJoinRequestMessage = null
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
        _uiState.value = _uiState.value.copy(
            listenerState = ListenerLifecycleState.ERROR,
            isScanning = false,
            lastError = message,
        )
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
        _uiState.value = _uiState.value.copy(
            hostState = HostLifecycleState.ERROR,
            hostPlaybackState = if (_uiState.value.hostPlaybackState == PlaybackState.PLAYING) {
                PlaybackState.ERROR
            } else {
                _uiState.value.hostPlaybackState
            },
            lastError = message,
        )
        diagnosticsStore.updateHost {
            it.copy(lastError = message, metricsSummary = summarizeMetrics())
        }
        refreshHostDiagnostics()
    }

    internal fun MainViewModel.handleControlMessage(message: ControlMessage) {
        when (message) {
            is ControlMessage.JoinRequest -> handleJoinRequestMessage(message)
            is ControlMessage.JoinApproval -> handleJoinApprovalMessage(message)
            is ControlMessage.JoinRejection -> handleJoinRejectionMessage(message)
            is ControlMessage.StreamStart -> handleRemoteStreamStart(message)
            is ControlMessage.Pause -> handleRemotePause(message)
            is ControlMessage.Stop -> handleRemoteStop(message)
            is ControlMessage.Disconnect -> handleRemoteDisconnect(message)
            is ControlMessage.Heartbeat -> wifiDirectService.recordHeartbeat()
            is ControlMessage.ResyncNotice -> {
                if (message.listenerId == localListenerDeviceId) {
                    _uiState.value = _uiState.value.copy(lastMessage = message.reason)
                }
            }
            is ControlMessage.Hello -> Unit
        }
    }

    internal fun MainViewModel.joinRejectionReason(message: ControlMessage.JoinRequest): String? {
        if (message.sessionId != currentSessionId) return "Session mismatch"
        val form = _uiState.value.hostForm
        if (form.approvalMode == ApprovalMode.INVITE_CODE) {
            val expected = form.inviteCode.trim()
            val actual = message.inviteCode?.trim().orEmpty()
            if (expected.isBlank()) return "Host invite code is not configured"
            if (actual != expected) return "Incorrect invite code"
        }
        return null
    }

    internal fun MainViewModel.handleJoinRequestMessage(message: ControlMessage.JoinRequest) {
        if (message.sessionId != currentSessionId) return

        val rejectionReason = joinRejectionReason(message)
        if (rejectionReason != null) {
            logger.w("listener.join.reject", rejectionReason)
            viewModelScope.launch {
                val delivered = runCatching {
                    wifiDirectService.broadcastControl(
                        ControlMessage.JoinRejection(
                            version = 1,
                            sessionId = message.sessionId,
                            listenerId = message.device.deviceId,
                            reason = rejectionReason,
                        ),
                    )
                }.map { result ->
                    reportHostBroadcastDelivery("send join rejection", result, requireAnyPeer = true)
                }.getOrElse { error ->
                    handleHostControlFailure("send join rejection", error)
                    false
                }

                if (delivered) {
                    val hostMessage = "Rejected ${message.device.displayName}: $rejectionReason"
                    diagnosticsStore.updateHost {
                        it.copy(lastError = hostMessage, metricsSummary = summarizeMetrics())
                    }
                    _uiState.value = _uiState.value.copy(lastError = hostMessage)
                    refreshHostDiagnostics()
                }
            }
            return
        }

        val request = JoinRequest(
            requestId = "${message.device.deviceId}-${message.sessionId.value}",
            sessionId = message.sessionId.value,
            listenerId = message.device.deviceId,
            listenerName = message.device.displayName,
            inviteCode = message.inviteCode,
            requestedAtMs = SystemClock.elapsedRealtime(),
        )
        if (_uiState.value.pendingJoinRequests.any { it.listenerId == request.listenerId }) return
        _uiState.value = _uiState.value.copy(
            pendingJoinRequests = _uiState.value.pendingJoinRequests + request,
            hostState = HostLifecycleState.READY,
            lastMessage = "${request.listenerName} requested to join",
            lastError = null,
        )
        refreshHostDiagnostics()
    }

    internal fun MainViewModel.handleJoinApprovalMessage(message: ControlMessage.JoinApproval) {
        if (message.listenerId != localListenerDeviceId) return
        _uiState.value = _uiState.value.copy(
            listenerState = ListenerLifecycleState.APPROVED,
            connectionProgress = _uiState.value.connectionProgress.copy(
                currentState = ListenerLifecycleState.APPROVED,
                approved = true,
                connected = true,
            ),
            lastMessage = if (message.trustedForFuture) {
                "Join approved; host remembered this device"
            } else {
                "Join approved"
            },
            lastError = null,
        )
        refreshListenerDiagnostics()
        requestListenerSyncProbe(source = "Initial clock sync")
    }

    internal fun MainViewModel.handleJoinRejectionMessage(message: ControlMessage.JoinRejection) {
        if (message.listenerId != localListenerDeviceId) return
        handleListenerConnectionFailure(message.reason)
    }

    internal fun MainViewModel.handleRemotePause(message: ControlMessage.Pause) {
        if (_uiState.value.selectedSession?.id != message.sessionId.value) return
        propagateListenerPlaybackState(
            playbackState = PlaybackState.PAUSED,
            listenerState = _uiState.value.listenerState,
            message = "Host paused the stream",
        )
    }

    internal fun MainViewModel.handleRemoteStop(message: ControlMessage.Stop) {
        if (_uiState.value.selectedSession?.id != message.sessionId.value) return
        playbackJob?.cancel()
        listenerScheduler = null
        pendingTransportPackets.clear()
        propagateListenerPlaybackState(
            playbackState = PlaybackState.STOPPED,
            listenerState = ListenerLifecycleState.CONNECTING,
            message = "Host stopped the stream",
        )
        diagnosticsStore.updateListener {
            it.copy(endOfStreamReached = true, playbackState = PlaybackState.STOPPED)
        }
        refreshListenerDiagnostics()
    }

    internal fun MainViewModel.handleRemoteDisconnect(message: ControlMessage.Disconnect) {
        if (_uiState.value.selectedSession?.id != message.sessionId.value) return
        if (message.listenerId != localListenerDeviceId && message.listenerId.isNotBlank()) return
        handleListenerDisconnect(message.reason)
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

    internal fun MainViewModel.handleListenerConnectionFailure(message: String) {
        clearScanState()
        logger.w("transport.error", message)
        playbackJob?.cancel()
        resyncJob?.cancel()
        playbackEngine.stop()
        _uiState.value = _uiState.value.copy(
            listenerState = ListenerLifecycleState.ERROR,
            listenerPlaybackState = PlaybackState.ERROR,
            connectionProgress = _uiState.value.connectionProgress.copy(
                buffered = false,
                playing = false,
            ),
            lastError = message,
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

    internal fun MainViewModel.handleListenerDisconnect(message: String) {
        clearScanState()
        playbackJob?.cancel()
        resyncJob?.cancel()
        playbackEngine.stop()
        listenerScheduler = null
        pendingTransportPackets.clear()
        pendingSyncCorrelationId = null
        pendingJoinRequestMessage = null
        logger.w("transport.disconnect", message)
        _uiState.value = _uiState.value.copy(
            listenerState = ListenerLifecycleState.DISCONNECTED,
            listenerPlaybackState = PlaybackState.STOPPED,
            connectionProgress = _uiState.value.connectionProgress.copy(
                buffered = false,
                playing = false,
            ),
            lastError = message,
        )
        diagnosticsStore.updateListener {
            it.copy(
                playbackState = PlaybackState.STOPPED,
                lastError = message,
                metricsSummary = summarizeMetrics(),
            )
        }
        refreshListenerDiagnostics()
    }

    internal fun MainViewModel.refreshDiscoveredSessions() {
        val bleSessions = bleService.discoveredSessions.value
        val peerSessions = wifiDirectService.snapshot.value.peers.map {
            SessionInfo(
                id = "p2p-${it.deviceAddress.replace(":", "").lowercase()}",
                name = "Nearby session (${it.deviceName})",
                hostDeviceName = it.deviceName,
                approvalMode = ApprovalMode.MANUAL,
                inviteCodeRequired = false,
            )
        }
        val merged = (bleSessions + peerSessions)
            .distinctBy { it.id }
            .sortedBy { it.name }
        _uiState.value = _uiState.value.copy(
            discoveredSessions = merged,
            connectionProgress = _uiState.value.connectionProgress.copy(discovered = merged.isNotEmpty()),
        )
    }

    internal fun MainViewModel.sendPendingJoinRequest() {
        val request = pendingJoinRequestMessage ?: return
        viewModelScope.launch {
            runCatching {
                wifiDirectService.sendControlToHost(request)
            }.onSuccess {
                pendingJoinRequestMessage = null
                _uiState.value = _uiState.value.copy(
                    listenerState = ListenerLifecycleState.AWAITING_APPROVAL,
                    connectionProgress = _uiState.value.connectionProgress.copy(
                        currentState = ListenerLifecycleState.AWAITING_APPROVAL,
                        connected = true,
                    ),
                    lastMessage = "Join request sent",
                    lastError = null,
                )
            }.onFailure { error ->
                handleListenerConnectionFailure(error.message ?: "Failed to send join request")
            }
        }
    }
