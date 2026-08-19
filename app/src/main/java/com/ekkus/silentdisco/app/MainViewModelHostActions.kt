package com.ekkus.silentdisco.app

import android.os.SystemClock
import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.core.audio.AudioDecodeResult
import com.ekkus.silentdisco.core.audio.AudioFormatSpec
import com.ekkus.silentdisco.core.audio.DecodedAudioChunk
import com.ekkus.silentdisco.core.audio.PcmPacketizer
import com.ekkus.silentdisco.core.audio.packetizationStats
import com.ekkus.silentdisco.core.audio.validatePacketBudget
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.transport.SendAllResult
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportDelivery
import com.ekkus.silentdisco.core.uniffi.FfiPlaybackState
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** A missing delivery (transport not yet bound) reports as zero peers reached. */
internal fun FfiHostTransportDelivery?.toSendAllResult(): SendAllResult = SendAllResult(
    peerCount = this?.intendedPeers?.toInt() ?: 0,
    successCount = this?.successfulPeers?.toInt() ?: 0,
    failureCount = this?.failedPeers?.toInt() ?: 0,
)

internal fun MainViewModel.startHostPlaybackImpl() {
    if (_uiState.value.hostPlaybackState == PlaybackState.PAUSED) {
        resumeHostPlaybackImpl()
        return
    }

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
    val sessionId = currentSessionId ?: return

    launchHostPlaybackCommand("start host playback") {
        afterAuthoritativeHostPlaybackState(FfiPlaybackState.BUFFERING) {
            diagnosticsStore.updateHost {
                it.copy(
                    streamState = PlaybackState.BUFFERING,
                    lastError = null,
                    metricsSummary = summarizeMetrics(),
                )
            }
            refreshHostDiagnostics(streamState = PlaybackState.BUFFERING)
        }

        val decoded = try {
            withContext(Dispatchers.IO) {
                latestDecodedAudio ?: decoder.decode(selectedAudio)
            }
        } catch (error: Throwable) {
            handleHostPlaybackPreparationFailure(error)
            return@launchHostPlaybackCommand
        }
        latestDecodedAudio = decoded

        val streamId = currentStreamId ?: StreamId("stream-${SystemClock.elapsedRealtime()}").also {
            currentStreamId = it
        }
        val prepared = try {
            prepareHostStream(decoded, sessionId, streamId)
        } catch (error: Throwable) {
            handleHostPlaybackPreparationFailure(error)
            return@launchHostPlaybackCommand
        }
        latestPackets = prepared.packets

        afterAuthoritativeHostPlaybackState(FfiPlaybackState.PLAYING) {
            val backend = try {
                withContext(Dispatchers.IO) {
                    startHostMonitorPlayback(
                        sessionId = sessionId,
                        streamId = streamId,
                        format = decoded.format,
                        firstPacket = prepared.packets.first(),
                    )
                }
            } catch (error: Throwable) {
                handleHostPlaybackEngineFailure(error)
                return@afterAuthoritativeHostPlaybackState
            }
            metrics.increment("stream_start")
            metrics.recordTiming("packet_duration_ms", 20.0)
            metrics.recordTiming("average_packet_bytes", prepared.averagePacketBytes)
            diagnosticsStore.updateHost {
                it.copy(
                    packetSendCount = 0,
                    packetSendRatePerSecond = 0.0,
                    packetBudgetSummary = prepared.packetBudgetSummary,
                    streamState = PlaybackState.PLAYING,
                    lastContactElapsedMs = SystemClock.elapsedRealtime(),
                    metricsSummary = summarizeMetrics(),
                    lastError = null,
                )
            }
            logger.i(
                "stream.start",
                "stream=${streamId.value} packets=${prepared.packets.size} " +
                    "budget=${prepared.packetBudgetSummary}",
            )
            _uiState.value = _uiState.value.copy(
                lastMessage = "Host stream started via $backend",
                lastError = null,
            )
            broadcastHostStreamStart(streamId, decoded.format)
            refreshHostDiagnostics(streamState = PlaybackState.PLAYING)
            startHostStreamingLoop(streamId)
        }
    }
}

private fun MainViewModel.resumeHostPlaybackImpl() {
    val sessionId = currentSessionId
    val streamId = currentStreamId
    val format = latestDecodedAudio?.format
    if (sessionId == null || streamId == null || format == null || latestPackets.isEmpty()) {
        val message = "Host stream cannot resume because prepared audio state is unavailable"
        _uiState.value = _uiState.value.copy(lastError = message)
        reportRustHostPlaybackState(PlaybackState.ERROR, message)
        return
    }

    launchHostPlaybackCommand("resume host playback") {
        afterAuthoritativeHostPlaybackState(FfiPlaybackState.PLAYING) {
            logger.i("stream.resume", "Resuming host stream")
            metrics.increment("stream_resume")
            broadcastHostStreamStart(streamId, format)
            _uiState.value = _uiState.value.copy(lastMessage = "Host stream resumed", lastError = null)
            refreshHostDiagnostics(streamState = PlaybackState.PLAYING)
        }
    }
}

private fun MainViewModel.broadcastHostStreamStart(
    streamId: StreamId,
    format: AudioFormatSpec,
) {
    viewModelScope.launch(Dispatchers.IO) {
        runCatching {
            hostTransportController.broadcastStreamStart(
                streamId = streamId.value,
                hostStartTimeMs = SystemClock.elapsedRealtime() + 100,
                sampleRate = format.sampleRate,
                channels = format.channelCount,
                samplesPerPacket = latestPackets.first().samplesPerPacket,
            )
        }.onSuccess { delivery ->
            reportHostBroadcastDelivery("broadcast stream start", delivery.toSendAllResult(), requireAnyPeer = false)
        }.onFailure { error ->
            handleHostControlFailure("broadcast stream start", error)
        }
    }
}

internal fun MainViewModel.pauseHostPlaybackImpl() {
    launchHostPlaybackCommand("pause host playback") {
        afterAuthoritativeHostPlaybackState(FfiPlaybackState.PAUSED) {
            logger.i("stream.pause", "Pausing host stream")
            metrics.increment("stream_pause")
            propagateListenerPlaybackState(
                playbackState = PlaybackState.PAUSED,
                listenerState = _uiState.value.listenerState,
                message = "Host paused the stream",
            )
            currentStreamId?.let { streamId ->
                viewModelScope.launch(Dispatchers.IO) {
                    runCatching {
                        hostTransportController.broadcastPause(
                            streamId = streamId.value,
                            hostPauseTimeMs = SystemClock.elapsedRealtime(),
                        )
                    }.onSuccess { delivery ->
                        val warning = hostControlDeliveryMessage("Paused", "broadcast pause", delivery.toSendAllResult())
                        if (warning != null) {
                            _uiState.value = _uiState.value.copy(lastError = warning)
                            diagnosticsStore.updateHost {
                                it.copy(lastError = warning, metricsSummary = summarizeMetrics())
                            }
                        } else {
                            diagnosticsStore.updateHost {
                                it.copy(lastError = null, metricsSummary = summarizeMetrics())
                            }
                        }
                        refreshHostDiagnostics()
                    }.onFailure { error ->
                        handleHostControlFailure("broadcast pause", error)
                    }
                }
            }
            refreshHostDiagnostics(streamState = PlaybackState.PAUSED)
        }
    }
}

internal fun MainViewModel.stopHostPlaybackImpl() {
    launchHostPlaybackCommand("stop host playback", cancelExisting = true) {
        afterAuthoritativeHostPlaybackState(FfiPlaybackState.STOPPED) {
            logger.i("stream.stop", "Stopping host stream")
            hostStreamJob?.cancel()
            playbackJob?.cancel()
            resyncJob?.cancel()
            try {
                withContext(Dispatchers.IO) { stopHostMonitorPlayback() }
            } catch (error: Throwable) {
                handleHostPlaybackEngineFailure(error)
                return@afterAuthoritativeHostPlaybackState
            }
            metrics.increment("stream_stop")
            propagateListenerPlaybackState(
                playbackState = PlaybackState.STOPPED,
                listenerState = _uiState.value.listenerState,
                message = "Host stopped the stream",
            )
            currentStreamId?.let { streamId ->
                viewModelScope.launch(Dispatchers.IO) {
                    runCatching {
                        hostTransportController.broadcastStop(
                            streamId = streamId.value,
                            hostStopTimeMs = SystemClock.elapsedRealtime(),
                        )
                    }.onSuccess { delivery ->
                        val warning = hostControlDeliveryMessage("Stopped", "broadcast stop", delivery.toSendAllResult())
                        if (warning != null) {
                            _uiState.value = _uiState.value.copy(lastError = warning)
                            diagnosticsStore.updateHost {
                                it.copy(lastError = warning, metricsSummary = summarizeMetrics())
                            }
                        } else {
                            diagnosticsStore.updateHost {
                                it.copy(lastError = null, metricsSummary = summarizeMetrics())
                            }
                        }
                        refreshHostDiagnostics()
                    }.onFailure { error ->
                        handleHostControlFailure("broadcast stop", error)
                    }
                }
            }
            refreshHostDiagnostics(streamState = PlaybackState.STOPPED)
        }
    }
}

private fun MainViewModel.launchHostPlaybackCommand(
    action: String,
    cancelExisting: Boolean = false,
    block: suspend () -> Unit,
) {
    val existing = hostPlaybackCommandJob
    if (cancelExisting) {
        existing?.cancel()
    } else if (existing?.isActive == true) {
        _uiState.value = _uiState.value.copy(
            lastError = "Another host playback command is already pending",
        )
        return
    }

    val job = viewModelScope.launch {
        try {
            block()
        } catch (error: CancellationException) {
            throw error
        } catch (error: Throwable) {
            reportHostPlaybackCommandFailure(action, error)
        }
    }
    hostPlaybackCommandJob = job
    job.invokeOnCompletion {
        if (hostPlaybackCommandJob === job) {
            hostPlaybackCommandJob = null
        }
    }
}

private suspend fun MainViewModel.afterAuthoritativeHostPlaybackState(
    state: FfiPlaybackState,
    afterAccepted: suspend () -> Unit,
) {
    runAfterAuthoritativeHostPlaybackTransition(
        target = state,
        transition = { target ->
            ensureRustHostCore().transitionPlaybackState(target)
            Unit
        },
        afterAccepted = afterAccepted,
    )
}

private fun MainViewModel.prepareHostStream(
    decoded: AudioDecodeResult,
    sessionId: SessionId,
    streamId: StreamId,
): PreparedHostStream {
    val totalBytes = decoded.chunks.sumOf { it.pcm16Le.size }
    val combinedBytes = ByteArray(totalBytes).also { buffer ->
        var offset = 0
        decoded.chunks.forEach { chunk ->
            chunk.pcm16Le.copyInto(buffer, offset)
            offset += chunk.pcm16Le.size
        }
    }
    val packetizer = PcmPacketizer(
        sessionId = sessionId,
        streamId = streamId,
        format = decoded.format,
    )
    val packets = packetizer.packetize(
        chunk = DecodedAudioChunk(
            pcm16Le = combinedBytes,
            firstSampleIndex = 0,
            frameCount = combinedBytes.size / decoded.format.bytesPerFrame,
        ),
        hostPresentationStartMs = SystemClock.elapsedRealtime() + 500,
    )
    if (packets.isEmpty()) {
        error("Decoded stream produced no playable packets")
    }
    val packetBudget = packets.validatePacketBudget()
    if (!packetBudget.valid) {
        error("Packet budget exceeded: ${packetBudget.maxPacketBytes} bytes")
    }
    val packetStats = packets.packetizationStats()
    return PreparedHostStream(
        packets = packets,
        packetBudgetSummary = packetBudget.summary(),
        averagePacketBytes = packetStats.averagePacketBytes,
    )
}

private fun MainViewModel.handleHostPlaybackPreparationFailure(error: Throwable) {
    metrics.increment("stream_start_error")
    logger.e("stream.start", "Failed to prepare host playback", error)
    val message = error.message ?: "Failed to decode audio file"
    _uiState.value = _uiState.value.copy(lastError = message)
    reportRustHostPlaybackState(PlaybackState.ERROR, message)
    diagnosticsStore.updateHost {
        it.copy(
            lastError = message,
            streamState = PlaybackState.ERROR,
            metricsSummary = summarizeMetrics(),
        )
    }
    refreshHostDiagnostics(streamState = PlaybackState.ERROR)
}

private fun MainViewModel.reportHostPlaybackCommandFailure(action: String, error: Throwable) {
    val message = error.message ?: "Failed to $action"
    logger.e("stream.authority", message, error)
    _uiState.value = _uiState.value.copy(lastError = message)
    diagnosticsStore.updateHost {
        it.copy(lastError = message, metricsSummary = summarizeMetrics())
    }
    refreshHostDiagnostics()
}

private data class PreparedHostStream(
    val packets: List<AudioPacket>,
    val packetBudgetSummary: String,
    val averagePacketBytes: Double,
)
