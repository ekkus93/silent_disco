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
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.sync.HostTimeMapper
import com.ekkus.silentdisco.core.sync.HostTimingService
import com.ekkus.silentdisco.core.sync.ClockSyncEstimator
import com.ekkus.silentdisco.core.sync.ListenerSyncController
import com.ekkus.silentdisco.core.sync.SyncMaintenanceConfig
import com.ekkus.silentdisco.core.transport.BleAdvertisement
import com.ekkus.silentdisco.core.transport.BleDiscoveryService
import com.ekkus.silentdisco.core.transport.BleOperation
import com.ekkus.silentdisco.core.transport.BroadcastDeliverySeverity
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

internal fun MainViewModel.startHostStreamingLoop(streamId: StreamId) {
    hostStreamJob?.cancel()
    hostStreamJob = viewModelScope.launch {
        var previousSendElapsedMs: Long? = null
        var consecutiveAudioSendFailures = 0
        var presentationOffsetMs = 0L
        var pauseStartedAtMs: Long? = null
        var authoritativePlaybackObserved = false
        val packetDurationMs = latestPackets.firstOrNull()?.let {
            it.samplesPerPacket * 1_000L / it.sampleRate
        } ?: 20L
        latestPackets.forEachIndexed { index, originalPacket ->
            while (!authoritativePlaybackObserved) {
                when (_uiState.value.hostPlaybackState) {
                    PlaybackState.PLAYING,
                    PlaybackState.PAUSED,
                    -> authoritativePlaybackObserved = true
                    PlaybackState.ERROR -> return@launch
                    PlaybackState.STOPPED,
                    PlaybackState.BUFFERING,
                    PlaybackState.READY,
                    PlaybackState.UNDERRUN,
                    -> delay(PAUSE_POLL_INTERVAL_MS)
                }
            }
            while (_uiState.value.hostPlaybackState == PlaybackState.PAUSED) {
                if (pauseStartedAtMs == null) {
                    pauseStartedAtMs = SystemClock.elapsedRealtime()
                }
                delay(PAUSE_POLL_INTERVAL_MS)
            }
            pauseStartedAtMs?.let { startedAt ->
                presentationOffsetMs += SystemClock.elapsedRealtime() - startedAt
                pauseStartedAtMs = null
                previousSendElapsedMs = null
            }
            if (_uiState.value.hostPlaybackState in setOf(PlaybackState.STOPPED, PlaybackState.ERROR)) {
                return@launch
            }

            val packet = if (presentationOffsetMs == 0L) {
                originalPacket
            } else {
                originalPacket.copy(
                    hostPresentationTimeMs = originalPacket.hostPresentationTimeMs + presentationOffsetMs,
                )
            }
            val now = SystemClock.elapsedRealtime()
            previousSendElapsedMs?.let { previous ->
                val sendGap = now - previous
                metrics.recordTiming("packet_gap_ms", sendGap.toDouble())
                if (kotlin.math.abs(sendGap - packetDurationMs) > 8) {
                    metrics.increment("packet_send_anomaly")
                    logger.w("packet.send.anomaly", "seq=${packet.sequenceNumber} gap=${sendGap}ms")
                }
            }
            if (index % 25 == 0 || index == latestPackets.lastIndex) {
                logger.i("stream.packet", "stream=${streamId.value} sent=${index + 1}/${latestPackets.size}")
            }
            metrics.increment("packet_send_total")
            diagnosticsStore.updateHost {
                it.copy(
                    packetSendCount = (index + 1).toLong(),
                    packetSendRatePerSecond = 1_000.0 / packetDurationMs,
                    lastContactElapsedMs = now,
                    metricsSummary = summarizeMetrics(),
                )
            }
            runCatching {
                playbackEngine.write(
                    PlaybackFrame(
                        packet = packet,
                        localDeadlineMs = packet.hostPresentationTimeMs,
                    ),
                )
            }.onFailure { error ->
                handleHostPlaybackEngineFailure(error)
                return@launch
            }
            runCatching {
                hostTransportController.broadcastAudio(
                    streamId = packet.streamId.value,
                    sequence = packet.sequenceNumber,
                    sampleRate = packet.sampleRate,
                    channels = packet.channelCount,
                    samplesPerPacket = packet.samplesPerPacket,
                    firstSampleIndex = packet.firstSampleIndex,
                    hostPresentationTimeMs = packet.hostPresentationTimeMs,
                    payload = packet.payload,
                ).toSendAllResult()
            }.onSuccess { result ->
                when {
                    result.peerCount == 0 -> {
                        consecutiveAudioSendFailures = 0
                        val message = "No connected listeners for audio broadcast"
                        _uiState.value = _uiState.value.copy(lastError = message)
                        diagnosticsStore.updateHost {
                            it.copy(lastError = message, metricsSummary = summarizeMetrics())
                        }
                        refreshHostDiagnostics()
                    }
                    result.failureCount > 0 -> {
                        consecutiveAudioSendFailures += 1
                        val message = "Audio packet delivered to ${result.successCount}/${result.peerCount} listeners; ${result.failureCount} failed"
                        logger.w("transport.audio", message)
                        _uiState.value = _uiState.value.copy(lastError = message)
                        diagnosticsStore.updateHost {
                            it.copy(lastError = message, metricsSummary = summarizeMetrics())
                        }
                        refreshHostDiagnostics()
                    }
                    else -> {
                        consecutiveAudioSendFailures = 0
                        diagnosticsStore.updateHost { it.copy(lastError = null) }
                    }
                }
            }.onFailure { error ->
                consecutiveAudioSendFailures += 1
                val message = error.message ?: "Failed to send audio packet"
                logger.w("transport.audio", "Failed to send packet ${packet.sequenceNumber}: $message")
                _uiState.value = _uiState.value.copy(lastError = message)
                diagnosticsStore.updateHost {
                    it.copy(lastError = message, metricsSummary = summarizeMetrics())
                }
                refreshHostDiagnostics()
            }
            if (consecutiveAudioSendFailures >= 10) {
                val message = "Audio transport failed repeatedly; stream stopped"
                hostStreamJob?.cancel()
                _uiState.value = _uiState.value.copy(lastError = message)
                reportRustHostPlaybackState(PlaybackState.ERROR, message)
                diagnosticsStore.updateHost {
                    it.copy(
                        streamState = PlaybackState.ERROR,
                        lastError = message,
                        metricsSummary = summarizeMetrics(),
                    )
                }
                refreshHostDiagnostics(streamState = PlaybackState.ERROR)
                return@launch
            }
            refreshHostDiagnostics()
            previousSendElapsedMs = now
            delay(packetDurationMs)
        }
        logger.i("stream.stop", "Reached end of file for host stream")
        metrics.increment("stream_eof")
        _uiState.value = _uiState.value.copy(lastMessage = "Reached end of file")
        reportRustHostPlaybackState(PlaybackState.STOPPED)
        propagateListenerPlaybackState(
            playbackState = PlaybackState.STOPPED,
            listenerState = _uiState.value.listenerState,
            message = "Host stream reached end of file",
        )
        viewModelScope.launch(Dispatchers.IO) {
            runCatching {
                hostTransportController.broadcastStop(
                    streamId = streamId.value,
                    hostStopTimeMs = SystemClock.elapsedRealtime(),
                )
            }.onSuccess { delivery ->
                reportHostBroadcastDelivery("broadcast stream stop", delivery.toSendAllResult(), requireAnyPeer = false)
            }.onFailure { error ->
                handleHostControlFailure("broadcast stream stop", error)
            }
        }
        refreshHostDiagnostics(streamState = PlaybackState.STOPPED)
    }
}

internal fun MainViewModel.handleHostPlaybackEngineFailure(error: Throwable) {
    val message = error.message ?: "Host playback engine failed"
    logger.e("playback.host", message, error)
    hostStreamJob?.cancel()
    _uiState.value = _uiState.value.copy(lastError = message)
    reportRustHostPlaybackState(PlaybackState.ERROR, message)
    diagnosticsStore.updateHost {
        it.copy(
            streamState = PlaybackState.ERROR,
            lastError = message,
            metricsSummary = summarizeMetrics(),
        )
    }
    refreshHostDiagnostics(streamState = PlaybackState.ERROR)
}

private const val PAUSE_POLL_INTERVAL_MS = 20L
