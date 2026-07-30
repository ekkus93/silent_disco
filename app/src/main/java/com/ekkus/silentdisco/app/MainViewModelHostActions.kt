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

internal fun MainViewModel.startHostPlaybackImpl() {
    val selectedAudio = _uiState.value.hostForm.selectedAudio
    if (selectedAudio == null) {
        _uiState.value = _uiState.value.copy(lastError = "Choose an audio file before starting playback")
        return
    }
    val sessionError = requireHostSessionForPlayback(currentSessionId)
    if (sessionError != null) {
        _uiState.value = _uiState.value.copy(lastError = sessionError)
        reportRustHostPlaybackState(PlaybackState.ERROR, sessionError)
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
            lastMessage = "Host stream started via $backend",
            lastError = null,
        )
        reportRustHostPlaybackState(PlaybackState.PLAYING)
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
        val message = error.message ?: "Failed to decode audio file"
        _uiState.value = _uiState.value.copy(lastError = message)
        reportRustHostPlaybackState(PlaybackState.ERROR, message)
        diagnosticsStore.updateHost { it.copy(lastError = error.message, streamState = PlaybackState.ERROR) }
        refreshHostDiagnostics(streamState = PlaybackState.ERROR)
    }
}

internal fun MainViewModel.pauseHostPlaybackImpl() {
    logger.i("stream.pause", "Pausing host stream")
    reportRustHostPlaybackState(PlaybackState.PAUSED)
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
    reportRustHostPlaybackState(PlaybackState.STOPPED)
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
