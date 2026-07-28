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

internal fun MainViewModel.createHostSessionImpl(): Boolean {
    if (!requirePersistenceReady("start a host session")) return false
    val validationError = validateHostForm(_uiState.value)
    if (validationError != null) {
        _uiState.value = _uiState.value.copy(
            hostState = HostLifecycleState.ERROR,
            lastError = validationError,
        )
        return false
    }

    if (!hasHostTransportPermissions()) {
        val message = "Missing nearby connectivity permissions for advertising"
        wifiDirectService.fail(message, retryable = true)
        _uiState.value = _uiState.value.copy(
            hostState = HostLifecycleState.ERROR,
            lastError = message,
        )
        refreshHostDiagnostics(streamState = PlaybackState.ERROR)
        return false
    }

    _uiState.value = _uiState.value.copy(
        hostState = HostLifecycleState.CREATING_SESSION,
        lastError = null,
        lastMessage = "Starting host session…",
    )
    val form = _uiState.value.hostForm
    currentSessionId = SessionId(UUID.randomUUID().toString())
    currentStreamId = StreamId("stream-${SystemClock.elapsedRealtime()}")
    val session = SessionInfo(
        id = currentSessionId!!.value,
        name = form.sessionName.trim(),
        hostDeviceName = "This Android Host",
        approvalMode = form.approvalMode,
        inviteCodeRequired = form.approvalMode == ApprovalMode.INVITE_CODE,
    )

    val bleAdvertiseResult = bleService.startAdvertising(
        BleAdvertisement(
            sessionId = session.id,
            sessionName = session.name,
            hostName = session.hostDeviceName,
            approvalRequired = true,
            inviteCodeRequired = session.inviteCodeRequired,
        ),
    )
    if (!bleAdvertiseResult.started) {
        val message = bleAdvertiseResult.message ?: "BLE advertising could not start"
        logger.w("transport.host", message)
        _uiState.value = _uiState.value.copy(
            hostState = HostLifecycleState.ERROR,
            lastError = message,
        )
        return false
    }

    val transportResult = runCatching {
        wifiDirectService.startHost(session)
    }.getOrElse { error ->
        val message = error.message ?: "Failed to start host session"
        logger.e("transport.host", message, error)
        bleService.stopAdvertising()
        wifiDirectService.fail(message, retryable = true)
        _uiState.value = _uiState.value.copy(
            hostState = HostLifecycleState.ERROR,
            lastError = message,
        )
        refreshHostDiagnostics(streamState = PlaybackState.ERROR, sessionId = session.id)
        return false
    }

    if (!transportResult.started) {
        val message = transportResult.message ?: "Wi-Fi Direct host could not start"
        logger.w("transport.host", message)
        bleService.stopAdvertising()
        _uiState.value = _uiState.value.copy(
            hostState = HostLifecycleState.ERROR,
            lastError = message,
        )
        refreshHostDiagnostics(streamState = PlaybackState.ERROR, sessionId = session.id)
        return false
    }

    logger.i("transport.host", "Started host session ${session.id}")
    diagnosticsStore.updateHost {
        it.copy(
            sessionId = session.id,
            streamState = PlaybackState.STOPPED,
            lastContactElapsedMs = SystemClock.elapsedRealtime(),
            lastError = null,
        )
    }
    _uiState.value = _uiState.value.copy(
        hostState = HostLifecycleState.WAITING_FOR_LISTENERS,
        discoveredSessions = listOf(session),
        lastMessage = "Hosting ${session.name}",
        lastError = null,
    )
    refreshHostDiagnostics()
    return true
}

internal fun MainViewModel.approveJoinRequestImpl(request: JoinRequest) {
    val sessionId = currentSessionId ?: run {
        _uiState.value = _uiState.value.copy(lastError = "No active host session")
        return
    }
    logger.i("approval.approve", "Approving ${request.listenerName}")
    viewModelScope.launch {
        val outcome = executeJoinApproval(
            rememberForFuture = _uiState.value.hostForm.rememberApprovedDevices,
            persistTrust = {
                persistTrustedListenerRecord(request.listenerId, request.listenerName)
            },
            sendApproval = { trustedForFuture ->
                runCatching {
                    wifiDirectService.broadcastControl(
                        ControlMessage.JoinApproval(
                            version = 1,
                            sessionId = sessionId,
                            listenerId = request.listenerId,
                            trustedForFuture = trustedForFuture,
                        ),
                    )
                }.map { result ->
                    reportHostBroadcastDelivery(
                        "send join approval",
                        result,
                        requireAnyPeer = true,
                    )
                }.getOrElse { error ->
                    handleHostControlFailure("send join approval", error)
                    false
                }
            },
        )

        if (!outcome.delivered) {
            outcome.persistenceError?.let { error ->
                logger.e(
                    "storage.trust",
                    "Trusted-device persistence also failed before approval delivery",
                    error,
                )
            }
            return@launch
        }

        val persistenceMessage = outcome.persistenceError
            ?.let(::trustedListenerPersistenceMessage)
        outcome.persistenceError?.let { error ->
            logger.e("storage.trust", persistenceMessage.orEmpty(), error)
        }
        _uiState.value = _uiState.value.copy(
            pendingJoinRequests = _uiState.value.pendingJoinRequests.filterNot {
                it.requestId == request.requestId
            },
            approvedListeners = (
                _uiState.value.approvedListeners +
                    request.toListenerInfo(outcome.trustState)
            ).distinctBy { it.deviceId },
            lastMessage = if (persistenceMessage == null) {
                "${request.listenerName} approved"
            } else {
                "${request.listenerName} approved for this session only"
            },
            lastError = persistenceMessage,
        )
        refreshHostDiagnostics()
    }
}

internal fun MainViewModel.rejectJoinRequestImpl(request: JoinRequest) {
    logger.w("approval.reject", "Rejecting ${request.listenerName}")
    viewModelScope.launch {
        val delivered = runCatching {
            wifiDirectService.broadcastControl(
                ControlMessage.JoinRejection(
                    version = 1,
                    sessionId = SessionId(request.sessionId),
                    listenerId = request.listenerId,
                    reason = "Host rejected ${request.listenerName}",
                ),
            )
        }.map { result ->
            reportHostBroadcastDelivery("send join rejection", result, requireAnyPeer = true)
        }.getOrElse { error ->
            handleHostControlFailure("send join rejection", error)
            false
        }

        if (!delivered) return@launch

        _uiState.value = _uiState.value.copy(
            pendingJoinRequests = _uiState.value.pendingJoinRequests.filterNot { it.requestId == request.requestId },
            lastMessage = "Rejected ${request.listenerName}",
            lastError = null,
        )
        diagnosticsStore.updateHost {
            it.copy(lastError = null, metricsSummary = summarizeMetrics())
        }
        refreshHostDiagnostics()
    }
}

internal fun MainViewModel.startHostPlaybackImpl() {
    val selectedAudio = _uiState.value.hostForm.selectedAudio
    if (selectedAudio == null) {
        _uiState.value = _uiState.value.copy(lastError = "Choose an audio file before starting playback")
        return
    }
    val sessionError = requireHostSessionForPlayback(currentSessionId)
    if (sessionError != null) {
        _uiState.value = _uiState.value.copy(
            hostState = HostLifecycleState.ERROR,
            hostPlaybackState = PlaybackState.ERROR,
            lastError = sessionError,
        )
        diagnosticsStore.updateHost {
            it.copy(
                streamState = PlaybackState.ERROR,
                lastError = sessionError,
                metricsSummary = summarizeMetrics(),
            )
        }
        refreshHostDiagnostics(streamState = PlaybackState.ERROR)
        return
    }
    val sessionId = currentSessionId!!
    val streamId = currentStreamId ?: StreamId("stream-${SystemClock.elapsedRealtime()}").also {
        currentStreamId = it
    }
    runCatching {
        latestDecodedAudio ?: decoder.decode(selectedAudio)
    }.onSuccess { decoded ->
        latestDecodedAudio = decoded
        val totalBytes = decoded.chunks.sumOf { it.pcm16Le.size }
        val combinedBytes = ByteArray(totalBytes).also { buf ->
            var offset = 0
            decoded.chunks.forEach { chunk ->
                chunk.pcm16Le.copyInto(buf, offset)
                offset += chunk.pcm16Le.size
            }
        }
        val packetizer = PcmPacketizer(
            sessionId = sessionId,
            streamId = streamId,
            format = decoded.format,
        )
        latestPackets = packetizer.packetize(
            chunk = DecodedAudioChunk(
                pcm16Le = combinedBytes,
                firstSampleIndex = 0,
                frameCount = combinedBytes.size / decoded.format.bytesPerFrame,
            ),
            hostPresentationStartMs = SystemClock.elapsedRealtime() + 500,
        )
        if (latestPackets.isEmpty()) {
            error("Decoded stream produced no playable packets")
        }
        val packetBudget = latestPackets.validatePacketBudget()
        val packetStats = latestPackets.packetizationStats()
        if (!packetBudget.valid) {
            error("Packet budget exceeded: ${packetBudget.maxPacketBytes} bytes")
        }
        metrics.increment("stream_start")
        metrics.recordTiming("packet_duration_ms", 20.0)
        metrics.recordTiming("average_packet_bytes", packetStats.averagePacketBytes)
        diagnosticsStore.updateHost {
            it.copy(
                packetSendCount = 0,
                packetSendRatePerSecond = 0.0,
                packetBudgetSummary = packetBudget.summary(),
                streamState = PlaybackState.PLAYING,
                lastContactElapsedMs = SystemClock.elapsedRealtime(),
                metricsSummary = summarizeMetrics(),
                lastError = null,
            )
        }
        val backend = runCatching { playbackEngine.start(decoded.format) }.getOrElse { error ->
            handleHostPlaybackEngineFailure(error)
            return@onSuccess
        }
        logger.i(
            "stream.start",
            "stream=${streamId.value} packets=${latestPackets.size} budget=${packetBudget.summary()}",
        )
        _uiState.value = _uiState.value.copy(
            hostState = HostLifecycleState.STREAMING,
            hostPlaybackState = PlaybackState.PLAYING,
            lastMessage = "Host stream started via $backend",
            lastError = null,
        )
        viewModelScope.launch {
            runCatching {
                wifiDirectService.broadcastControl(
                    ControlMessage.StreamStart(
                        version = 1,
                        sessionId = sessionId,
                        streamId = streamId,
                        hostStartTimeMs = latestPackets.first().hostPresentationTimeMs,
                        sampleRate = decoded.format.sampleRate,
                        channels = decoded.format.channelCount,
                        samplesPerPacket = latestPackets.first().samplesPerPacket,
                    ),
                )
            }.onSuccess { result ->
                reportHostBroadcastDelivery("broadcast stream start", result, requireAnyPeer = false)
            }.onFailure { error ->
                handleHostControlFailure("broadcast stream start", error)
            }
        }
        refreshHostDiagnostics()
        startHostStreamingLoop(streamId)
    }.onFailure { error ->
        metrics.increment("stream_start_error")
        logger.e("stream.start", "Failed to start host playback", error)
        _uiState.value = _uiState.value.copy(
            hostState = HostLifecycleState.ERROR,
            hostPlaybackState = PlaybackState.ERROR,
            lastError = error.message ?: "Failed to decode audio file",
        )
        diagnosticsStore.updateHost { it.copy(lastError = error.message, streamState = PlaybackState.ERROR) }
        refreshHostDiagnostics(streamState = PlaybackState.ERROR)
    }
}

internal fun MainViewModel.pauseHostPlaybackImpl() {
    logger.i("stream.pause", "Pausing host stream")
    _uiState.value = _uiState.value.copy(
        hostState = HostLifecycleState.PAUSED,
        hostPlaybackState = PlaybackState.PAUSED,
    )
    metrics.increment("stream_pause")
    propagateListenerPlaybackState(
        playbackState = PlaybackState.PAUSED,
        listenerState = _uiState.value.listenerState,
        message = "Host paused the stream",
    )
    currentSessionId?.let { sessionId ->
        currentStreamId?.let { streamId ->
            viewModelScope.launch {
                runCatching {
                    wifiDirectService.broadcastControl(
                        ControlMessage.Pause(
                            version = 1,
                            sessionId = sessionId,
                            streamId = streamId,
                            hostPauseTimeMs = SystemClock.elapsedRealtime(),
                        ),
                    )
                }.onSuccess { result ->
                    val warning = hostControlDeliveryMessage("Paused", "broadcast pause", result)
                    if (warning != null) {
                        _uiState.value = _uiState.value.copy(lastError = warning)
                        diagnosticsStore.updateHost { it.copy(lastError = warning, metricsSummary = summarizeMetrics()) }
                    } else {
                        diagnosticsStore.updateHost { it.copy(lastError = null, metricsSummary = summarizeMetrics()) }
                    }
                    refreshHostDiagnostics()
                }.onFailure { error ->
                    handleHostControlFailure("broadcast pause", error)
                }
            }
        }
    }
    refreshHostDiagnostics(streamState = PlaybackState.PAUSED)
}

internal fun MainViewModel.stopHostPlaybackImpl() {
    logger.i("stream.stop", "Stopping host stream")
    hostStreamJob?.cancel()
    playbackJob?.cancel()
    resyncJob?.cancel()
    playbackEngine.stop()
    _uiState.value = _uiState.value.copy(
        hostState = HostLifecycleState.READY,
        hostPlaybackState = PlaybackState.STOPPED,
    )
    metrics.increment("stream_stop")
    propagateListenerPlaybackState(
        playbackState = PlaybackState.STOPPED,
        listenerState = _uiState.value.listenerState,
        message = "Host stopped the stream",
    )
    currentSessionId?.let { sessionId ->
        currentStreamId?.let { streamId ->
            viewModelScope.launch {
                runCatching {
                    wifiDirectService.broadcastControl(
                        ControlMessage.Stop(
                            version = 1,
                            sessionId = sessionId,
                            streamId = streamId,
                            hostStopTimeMs = SystemClock.elapsedRealtime(),
                        ),
                    )
                }.onSuccess { result ->
                    val warning = hostControlDeliveryMessage("Stopped", "broadcast stop", result)
                    if (warning != null) {
                        _uiState.value = _uiState.value.copy(lastError = warning)
                        diagnosticsStore.updateHost { it.copy(lastError = warning, metricsSummary = summarizeMetrics()) }
                    } else {
                        diagnosticsStore.updateHost { it.copy(lastError = null, metricsSummary = summarizeMetrics()) }
                    }
                    refreshHostDiagnostics()
                }.onFailure { error ->
                    handleHostControlFailure("broadcast stop", error)
                }
            }
        }
    }
    refreshHostDiagnostics(streamState = PlaybackState.STOPPED)
}

internal fun MainViewModel.endSessionImpl() {
    currentSessionId?.let { sessionId ->
        viewModelScope.launch {
            runCatching {
                wifiDirectService.broadcastControl(
                    ControlMessage.Disconnect(
                        version = 1,
                        sessionId = sessionId,
                        listenerId = localListenerDeviceId,
                        reason = "Host ended the session",
                    ),
                )
            }.onSuccess { result ->
                val warning = hostControlDeliveryMessage("Ended session", "broadcast session end", result)
                if (warning != null) {
                    _uiState.value = _uiState.value.copy(lastError = warning)
                    diagnosticsStore.updateHost { it.copy(lastError = warning, metricsSummary = summarizeMetrics()) }
                    refreshHostDiagnostics()
                }
            }.onFailure { error ->
                handleHostControlFailure("broadcast session end", error)
            }
        }
    }
    stopHostPlayback()
    bleService.stop()
    wifiDirectService.stop()
    logger.i("session.end", "Session ended by host")
    _uiState.value = _uiState.value.copy(
        hostState = HostLifecycleState.IDLE,
        pendingJoinRequests = emptyList(),
        approvedListeners = emptyList(),
        hostPlaybackState = PlaybackState.STOPPED,
        lastMessage = "Session ended",
    )
    if (_uiState.value.selectedSession != null) {
        _uiState.value = _uiState.value.copy(
            listenerState = ListenerLifecycleState.DISCONNECTED,
            listenerPlaybackState = PlaybackState.STOPPED,
            lastError = "Host ended the session",
        )
    }
    refreshHostDiagnostics(streamState = PlaybackState.STOPPED, sessionId = "")
}
