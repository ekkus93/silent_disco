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
import com.ekkus.silentdisco.core.model.SyncState
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.model.TrustState
import com.ekkus.silentdisco.core.permissions.PermissionCatalogue
import com.ekkus.silentdisco.core.permissions.AppPermission
import com.ekkus.silentdisco.core.permissions.PermissionState
import com.ekkus.silentdisco.core.rust.RustStoredTuningSettings
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.SessionId
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

    internal fun MainViewModel.startListenerPlaybackSimulation(sessionId: String) {
        val packets = if (latestPackets.isNotEmpty()) {
            latestPackets.take(24)
        } else {
            generateSyntheticPackets(sessionId)
        }
        val expectedStreamId = packets.firstOrNull()?.streamId ?: currentStreamId ?: StreamId("synthetic-stream")
        val mapper = HostTimeMapper(offsetMs = _uiState.value.listenerSyncState.offsetMs, skewPpm = _uiState.value.listenerSyncState.skewPpm)
        listenerScheduler = ListenerPlaybackScheduler(
            mapper = mapper,
            thresholds = currentPlaybackThresholds(),
            expectedSessionId = SessionId(sessionId),
            expectedStreamId = expectedStreamId,
        )
        packets.forEach { packet -> listenerScheduler?.let { recordIncomingPacket(it, packet) } }
        val playbackFormat = latestDecodedAudio?.format ?: AudioFormatSpec()
        runCatching { playbackEngine.start(playbackFormat) }.onFailure { error ->
            handleListenerPlaybackEngineFailure(error)
            return
        }
        playbackJob?.cancel()
        playbackJob = viewModelScope.launch {
            var lastUnderrunCount = 0
            delay(300)
            var playingStateSet = false
            while (listenerScheduler?.canStart() == true) {
                if (!playingStateSet) {
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
                    playingStateSet = true
                }
                if (wifiDirectService.snapshot.value.state == TransportConnectionState.DISCONNECTED ||
                    wifiDirectService.snapshot.value.state == TransportConnectionState.FAILED
                ) {
                    handleListenerDisconnect("Transport disconnected during playback")
                    return@launch
                }
                val frame = listenerScheduler?.poll() ?: break
                runCatching { playbackEngine.write(frame) }.onFailure { error ->
                    handleListenerPlaybackEngineFailure(error)
                    return@launch
                }
                val telemetry = listenerScheduler?.snapshot() ?: break
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
                delay(20)
            }
            diagnosticsStore.updateListener {
                it.copy(
                    playbackState = PlaybackState.STOPPED,
                    endOfStreamReached = true,
                    metricsSummary = summarizeMetrics(),
                )
            }
            _uiState.value = _uiState.value.copy(
                listenerPlaybackState = PlaybackState.STOPPED,
                lastMessage = "Reached end of file",
            )
            refreshListenerDiagnostics()
        }
        startPeriodicListenerResync()
    }

    internal fun MainViewModel.propagateListenerPlaybackState(
        playbackState: PlaybackState,
        listenerState: ListenerLifecycleState,
        message: String,
    ) {
        val isPlaying = playbackState == PlaybackState.PLAYING
        _uiState.value = _uiState.value.copy(
            listenerPlaybackState = playbackState,
            listenerState = listenerState,
            connectionProgress = _uiState.value.connectionProgress.copy(
                playing = isPlaying,
                buffered = if (!isPlaying) false else _uiState.value.connectionProgress.buffered,
            ),
            lastMessage = message,
        )
        diagnosticsStore.updateListener {
            it.copy(
                playbackState = playbackState,
                metricsSummary = summarizeMetrics(),
                lastError = if (listenerState == ListenerLifecycleState.DESYNCED) {
                    "Listener sync trouble detected"
                } else {
                    it.lastError
                },
            )
        }
        refreshListenerDiagnostics()
    }

    internal fun MainViewModel.handleListenerPlaybackEngineFailure(error: Throwable) {
        val message = error.message ?: "Playback engine failed"
        logger.e("playback.listener", message, error)
        playbackJob?.cancel()
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

    internal fun MainViewModel.generateSyntheticPackets(sessionId: String): List<AudioPacket> {
        val packetizer = PcmPacketizer(
            sessionId = SessionId(sessionId),
            streamId = StreamId("synthetic-stream"),
            format = AudioFormatSpec(),
        )
        return packetizer.packetize(
            chunk = DecodedAudioChunk(
                pcm16Le = ByteArray(48_000 / 25 * 4 * 8),
                firstSampleIndex = 0,
                frameCount = 48_000 / 25 * 8,
            ),
            hostPresentationStartMs = SystemClock.elapsedRealtime() + 500,
        )
    }
