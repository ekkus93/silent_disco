package com.ekkus.silentdisco.app

import android.os.SystemClock
import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.core.audio.AudioFormatSpec
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistFrame
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistStreamStart
import com.ekkus.silentdisco.core.uniffi.FfiPlaybackState
import com.ekkus.silentdisco.core.rust.HostPlaylistStreamController
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

internal fun MainViewModel.startHostPlaybackImpl() {
    val currentPlaybackState = _uiState.value.hostPlaybackState
    if (currentPlaybackState == PlaybackState.PAUSED) {
        resumeHostPlaybackImpl()
        return
    }
    if (currentPlaybackState in setOf(PlaybackState.PLAYING, PlaybackState.BUFFERING, PlaybackState.UNDERRUN)) {
        _uiState.value = _uiState.value.copy(lastError = "Stop the current stream before starting it again")
        return
    }

    if (_uiState.value.hostQueue.isEmpty() && _uiState.value.hostForm.selectedAudio == null) {
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

        val streamId = StreamId("stream-${SystemClock.elapsedRealtime()}-${java.util.UUID.randomUUID()}")
        hostPendingTrackBoundaries.clear()
        currentStreamId = streamId
        val streamer = try {
            val sources = withContext(Dispatchers.IO) { stageHostPlaylistSources() }
            withContext(Dispatchers.IO) {
                ensureRustHostCore().openHostPlaylist(
                    sessionId = sessionId.value,
                    streamId = streamId.value,
                    hostStartTimeMs = (SystemClock.elapsedRealtime() + HOST_STREAM_START_LEAD_MS).toULong(),
                    sources = sources,
                )
            }
        } catch (error: Throwable) {
            handleHostPlaybackPreparationFailure(error)
            return@launchHostPlaybackCommand
        }
        hostPlaylistStreamer = streamer
        hostPlaylistStreamStart = streamer.streamStart
        val streamStart = streamer.streamStart
        val format = AudioFormatSpec(streamStart.sampleRate.toInt(), streamStart.channels.toInt())
        hostPlaylistFormat = format
        val firstPacket = try {
            withContext(Dispatchers.IO) { streamer.nextAudioPacket(sessionId, streamId) }
        } catch (error: Throwable) {
            clearHostPlaylistStream()
            handleHostPlaybackPreparationFailure(error)
            return@launchHostPlaybackCommand
        }
        latestPackets = listOf(firstPacket)
        latestDecodedAudio = null
        val packetBudgetSummary = firstPacket.let { packet ->
            "${packet.payload.size + 48}B first packet; native streaming decoder"
        }

        afterAuthoritativeHostPlaybackStart(streamId) {
            try {
                withContext(Dispatchers.IO) {
                    startHostMonitorPlayback(
                        sessionId = sessionId,
                        streamId = streamId,
                        format = format,
                        firstPacket = firstPacket,
                    )
                }
            } catch (error: Throwable) {
                handleHostPlaybackEngineFailure(error)
                return@afterAuthoritativeHostPlaybackStart
            }
            metrics.increment("stream_start")
            metrics.recordTiming(
                "packet_duration_ms",
                firstPacket.samplesPerPacket * 1_000.0 / firstPacket.sampleRate,
            )
            metrics.recordTiming("average_packet_bytes", firstPacket.payload.size + 48.0)
            diagnosticsStore.updateHost {
                it.copy(
                    packetSendCount = 0,
                    packetSendRatePerSecond = 0.0,
                    packetBudgetSummary = packetBudgetSummary,
                    streamState = PlaybackState.PLAYING,
                    lastContactElapsedMs = SystemClock.elapsedRealtime(),
                    metricsSummary = summarizeMetrics(),
                    lastError = null,
                )
            }
            logger.i(
                "stream.start",
                "stream=${streamId.value} playlist_entries=${_uiState.value.hostQueue.size} " +
                    "budget=$packetBudgetSummary",
            )
            _uiState.value = _uiState.value.copy(
                lastMessage = "Playlist is playing",
                lastError = null,
            )
            broadcastHostStreamStart(streamId, streamStart)
            refreshHostDiagnostics(streamState = PlaybackState.PLAYING)
            startHostStreamingLoop(streamId, streamer, firstPacket, format)
        }
    }
}

private fun MainViewModel.resumeHostPlaybackImpl() {
    val sessionId = currentSessionId
    val streamId = currentStreamId
    val format = hostPlaylistFormat
    val streamStart = hostPlaylistStreamStart
    val streamer = hostPlaylistStreamer
    if (sessionId == null || streamId == null || format == null || streamStart == null || streamer == null) {
        val message = "Host stream cannot resume because prepared audio state is unavailable"
        _uiState.value = _uiState.value.copy(lastError = message)
        reportRustHostPlaybackFailure(message)
        return
    }

    launchHostPlaybackCommand("resume host playback") {
        afterAuthoritativeHostPlaybackState(FfiPlaybackState.PLAYING) {
            logger.i("stream.resume", "Resuming host stream")
            metrics.increment("stream_resume")
            _uiState.value = _uiState.value.copy(lastMessage = "Host stream resumed", lastError = null)
            refreshHostDiagnostics(streamState = PlaybackState.PLAYING)
        }
    }
}

internal suspend fun MainViewModel.broadcastHostStreamStart(
    streamId: StreamId,
    start: FfiPlaylistStreamStart,
) {
    runCatching {
        hostTransportController.broadcastStreamStart(
            streamId = streamId.value,
            hostStartTimeMs = start.hostStartTimeMs.toLong(),
            sampleRate = start.sampleRate.toInt(),
            channels = start.channels.toInt(),
            samplesPerPacket = start.samplesPerPacket.toInt(),
            currentEntryId = start.currentEntryId,
            currentDisplayName = start.currentDisplayName,
        )
    }.onSuccess { delivery ->
        reportHostBroadcastDelivery("broadcast stream start", delivery.toSendAllResult(), requireAnyPeer = false)
    }.onFailure { error ->
        handleHostControlFailure("broadcast stream start", error)
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
    // Boundary jobs are independent of the stream coroutine. Cancel them before
    // submitting STOP so none can apply a stale queue transition during teardown.
    cancelHostBoundaryJobs()
    launchHostPlaybackCommand("stop host playback", cancelExisting = true) {
        afterAuthoritativeHostPlaybackState(FfiPlaybackState.STOPPED) {
            logger.i("stream.stop", "Stopping host stream")
            hostStreamJob?.cancel()
            playbackJob?.cancel()
            resyncJob?.cancel()
            try {
                withContext(Dispatchers.IO) {
                    clearHostPlaylistStream()
                    stopHostMonitorPlayback()
                }
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

// The actor accepts PositionAdvanced(0) from Buffering as a single
// Playing-plus-stream-identity transition. Keep the UI state transition
// semantics while avoiding a two-event race against a delayed old EOF.
private suspend fun MainViewModel.afterAuthoritativeHostPlaybackStart(
    streamId: StreamId,
    afterAccepted: suspend () -> Unit,
) {
    runAfterAuthoritativeHostPlaybackTransition(
        target = FfiPlaybackState.PLAYING,
        transition = {
            ensureRustHostCore().playbackPositionAdvanced(streamId.value, 0L)
            Unit
        },
        afterAccepted = afterAccepted,
    )
}

private fun MainViewModel.handleHostPlaybackPreparationFailure(error: Throwable) {
    metrics.increment("stream_start_error")
    logger.e("stream.start", "Failed to prepare host playback", error)
    val message = error.message ?: "Failed to decode audio file"
    _uiState.value = _uiState.value.copy(lastError = message)
    reportRustHostPlaybackFailure(message)
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

private const val HOST_STREAM_START_LEAD_MS = 500L

private suspend fun HostPlaylistStreamController.nextAudioPacket(
    sessionId: SessionId,
    streamId: StreamId,
): AudioPacket {
    while (true) {
        when (val frame = nextFrame(2_000uL)) {
            null -> continue
            FfiPlaylistFrame.End -> error("Playlist ended before producing audio")
            is FfiPlaylistFrame.TrackBoundary -> error("Playlist began with an unexpected track boundary")
            is FfiPlaylistFrame.Audio -> return AudioPacket(
                version = 3,
                sessionId = sessionId,
                streamId = StreamId(frame.streamId),
                sequenceNumber = frame.sequence.toLong(),
                codec = "pcm16le",
                sampleRate = frame.sampleRate.toInt(),
                channelCount = frame.channels.toInt(),
                samplesPerPacket = frame.samplesPerPacket.toInt(),
                firstSampleIndex = frame.firstSampleIndex.toLong(),
                hostPresentationTimeMs = frame.hostPresentationTimeMs.toLong(),
                payload = frame.payload,
            ).also {
                require(it.streamId == streamId) { "playlist frame belongs to another stream" }
            }
        }
    }
}
