package com.ekkus.silentdisco.app

import android.os.SystemClock
import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.core.audio.AudioFormatSpec
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.rust.HostPlaylistStreamController
import com.ekkus.silentdisco.core.transport.SendAllResult
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportDelivery
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistFrame
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistStreamStart
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlin.math.min

/** A missing delivery (transport not yet bound) reports as zero peers reached. */
internal fun FfiHostTransportDelivery?.toSendAllResult(): SendAllResult = SendAllResult(
    peerCount = this?.intendedPeers?.toInt() ?: 0,
    successCount = this?.successfulPeers?.toInt() ?: 0,
    failureCount = this?.failedPeers?.toInt() ?: 0,
)

internal fun MainViewModel.startHostStreamingLoop(
    streamId: StreamId,
    streamer: HostPlaylistStreamController,
    firstPacket: AudioPacket,
    format: AudioFormatSpec,
) {
    hostStreamJob?.cancel()
    hostStreamJob = viewModelScope.launch(Dispatchers.IO) {
        var packet = firstPacket
        var pendingFirstPacket: AudioPacket? = firstPacket
        var packetCount = 0L
        var consecutiveAudioSendFailures = 0
        var presentationOffsetMs = 0L
        var lastReportedPositionMs = -POSITION_REPORT_INTERVAL_MS
        var pauseStartedAtMs: Long? = null
        var previousSendElapsedMs: Long? = null
        var lastPacket: AudioPacket? = null
        val start = hostPlaylistStreamStart ?: streamer.streamStart
        val packetDurationMs = firstPacket.samplesPerPacket * 1_000L / firstPacket.sampleRate
        var authoritativePlaybackObserved = false

        try {
            while (isActive) {
                while (!authoritativePlaybackObserved) {
                    when (_uiState.value.hostPlaybackState) {
                        PlaybackState.PLAYING, PlaybackState.PAUSED -> authoritativePlaybackObserved = true
                        PlaybackState.ERROR -> return@launch
                        PlaybackState.STOPPED,
                        PlaybackState.BUFFERING,
                        PlaybackState.READY,
                        PlaybackState.UNDERRUN,
                        -> delay(PAUSE_POLL_INTERVAL_MS)
                    }
                }
                while (_uiState.value.hostPlaybackState == PlaybackState.PAUSED) {
                    if (pauseStartedAtMs == null) pauseStartedAtMs = SystemClock.elapsedRealtime()
                    delay(PAUSE_POLL_INTERVAL_MS)
                }
                pauseStartedAtMs?.let { pausedAt ->
                    presentationOffsetMs += SystemClock.elapsedRealtime() - pausedAt
                    pauseStartedAtMs = null
                    previousSendElapsedMs = null
                    runCatching {
                        reanchorHostMonitorPlayback(
                            (lastPacket?.hostPresentationTimeMs ?: firstPacket.hostPresentationTimeMs) +
                                presentationOffsetMs,
                        )
                    }.onFailure { error ->
                        handleHostPlaybackEngineFailure(error)
                        return@launch
                    }
                    val resumedStart = start.copy(
                        hostStartTimeMs = start.hostStartTimeMs + presentationOffsetMs.toULong(),
                        currentEntryId = _uiState.value.hostQueue.firstOrNull()?.entryId,
                        currentDisplayName = _uiState.value.hostQueue.firstOrNull()?.displayName,
                    )
                    hostPlaylistStreamStart = resumedStart
                    broadcastHostStreamStart(streamId, resumedStart)
                    hostPendingTrackBoundaries.values
                        .sortedBy(PendingHostTrackBoundary::firstSampleIndex)
                        .forEach { pending ->
                            broadcastHostTrackBoundary(
                                streamId,
                                FfiPlaylistFrame.TrackBoundary(
                                    streamId = pending.streamId,
                                    firstSampleIndex = pending.firstSampleIndex.toULong(),
                                    entryId = pending.entryId,
                                    displayName = pending.displayName,
                                ),
                            )
                        }
                }
                if (_uiState.value.hostPlaybackState in setOf(PlaybackState.STOPPED, PlaybackState.ERROR)) {
                    return@launch
                }

                val prefetchedPacket = pendingFirstPacket
                if (prefetchedPacket != null) {
                    packet = prefetchedPacket
                    pendingFirstPacket = null
                } else {
                    when (val frame = withContext(Dispatchers.IO) { streamer.nextFrame(3uL) }) {
                        null -> continue
                        FfiPlaylistFrame.End -> break
                        is FfiPlaylistFrame.TrackBoundary -> {
                            ensureRustHostCore().trackBoundaryScheduled(
                                frame.entryId,
                                frame.firstSampleIndex.toLong(),
                            )
                            hostPendingTrackBoundaries[frame.firstSampleIndex.toLong()] =
                                PendingHostTrackBoundary(
                                    streamId = frame.streamId,
                                    firstSampleIndex = frame.firstSampleIndex.toLong(),
                                    entryId = frame.entryId,
                                    displayName = frame.displayName,
                                )
                            broadcastHostTrackBoundary(streamId, frame)
                            scheduleHostQueueBoundary(start, frame, format.sampleRate)
                            continue
                        }
                        is FfiPlaylistFrame.Audio -> packet = AudioPacket(
                            version = 3,
                            sessionId = firstPacket.sessionId,
                            streamId = StreamId(frame.streamId),
                            sequenceNumber = frame.sequence.toLong(),
                            codec = "pcm16le",
                            sampleRate = frame.sampleRate.toInt(),
                            channelCount = frame.channels.toInt(),
                            samplesPerPacket = frame.samplesPerPacket.toInt(),
                            firstSampleIndex = frame.firstSampleIndex.toLong(),
                            hostPresentationTimeMs = frame.hostPresentationTimeMs.toLong(),
                            payload = frame.payload,
                        )
                    }
                }

                val timedPacket = if (presentationOffsetMs == 0L) packet else packet.copy(
                    hostPresentationTimeMs = packet.hostPresentationTimeMs + presentationOffsetMs,
                )
                val now = SystemClock.elapsedRealtime()
                previousSendElapsedMs?.let { previous ->
                    val sendGap = now - previous
                    metrics.recordTiming("packet_gap_ms", sendGap.toDouble())
                    if (kotlin.math.abs(sendGap - packetDurationMs) > 8) {
                        metrics.increment("packet_send_anomaly")
                        logger.w("packet.send.anomaly", "seq=${packet.sequenceNumber} gap=${sendGap}ms")
                    }
                }
                packetCount += 1
                if (packetCount % 25L == 1L) {
                    logger.i("stream.packet", "stream=${streamId.value} sent=$packetCount")
                }
                metrics.increment("packet_send_total")
                diagnosticsStore.updateHost {
                    it.copy(
                        packetSendCount = packetCount,
                        packetSendRatePerSecond = 1_000.0 / packetDurationMs,
                        lastContactElapsedMs = now,
                        metricsSummary = summarizeMetrics(),
                    )
                }
                val playbackPositionMs = timedPacket.firstSampleIndex * 1_000L / timedPacket.sampleRate
                if (playbackPositionMs >= lastReportedPositionMs + POSITION_REPORT_INTERVAL_MS) {
                    runCatching {
                        ensureRustHostCore().playbackPositionAdvanced(streamId.value, playbackPositionMs)
                    }.onFailure { error ->
                        logger.w("playback.position", error.message ?: "Could not report host playback position")
                    }
                    lastReportedPositionMs = playbackPositionMs
                }
                runCatching { submitHostMonitorPacket(timedPacket) }
                    .onFailure { error ->
                        if (_uiState.value.hostPlaybackState != PlaybackState.STOPPED) {
                            handleHostPlaybackEngineFailure(error)
                        }
                        return@launch
                    }
                runCatching {
                    hostTransportController.broadcastAudio(
                        streamId = timedPacket.streamId.value,
                        sequence = timedPacket.sequenceNumber,
                        sampleRate = timedPacket.sampleRate,
                        channels = timedPacket.channelCount,
                        samplesPerPacket = timedPacket.samplesPerPacket,
                        firstSampleIndex = timedPacket.firstSampleIndex,
                        hostPresentationTimeMs = timedPacket.hostPresentationTimeMs,
                        payload = timedPacket.payload,
                    ).toSendAllResult()
                }.onSuccess { result ->
                    when {
                        result.peerCount == 0 -> {
                            consecutiveAudioSendFailures = 0
                            val message = "No connected listeners for audio broadcast"
                            _uiState.value = _uiState.value.copy(lastError = message)
                            diagnosticsStore.updateHost { it.copy(lastError = message, metricsSummary = summarizeMetrics()) }
                            refreshHostDiagnostics()
                        }
                        result.failureCount > 0 -> {
                            consecutiveAudioSendFailures += 1
                            val message = "Audio packet delivered to ${result.successCount}/${result.peerCount} listeners; ${result.failureCount} failed"
                            logger.w("transport.audio", message)
                            _uiState.value = _uiState.value.copy(lastError = message)
                            diagnosticsStore.updateHost { it.copy(lastError = message, metricsSummary = summarizeMetrics()) }
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
                    logger.w("transport.audio", "Failed to send packet ${timedPacket.sequenceNumber}: $message")
                    _uiState.value = _uiState.value.copy(lastError = message)
                    diagnosticsStore.updateHost { it.copy(lastError = message, metricsSummary = summarizeMetrics()) }
                    refreshHostDiagnostics()
                }
                if (consecutiveAudioSendFailures >= 10) {
                    val message = "Audio transport failed repeatedly; stream stopped"
                    _uiState.value = _uiState.value.copy(lastError = message)
                    reportRustHostPlaybackFailure(message)
                    diagnosticsStore.updateHost {
                        it.copy(streamState = PlaybackState.ERROR, lastError = message, metricsSummary = summarizeMetrics())
                    }
                    refreshHostDiagnostics(streamState = PlaybackState.ERROR)
                    return@launch
                }
                refreshHostDiagnostics()
                previousSendElapsedMs = now
                lastPacket = packet
                delay(packetDurationMs)
            }

            val finalPacket = lastPacket
            if (finalPacket != null) {
                awaitHostPacketPresentationTail(
                    finalPacket.hostPresentationTimeMs,
                    presentationOffsetMs,
                    packetDurationMs,
                )
            }
            stopHostMonitorPlayback()
            logger.i("stream.stop", "Host playlist reached its final track boundary")
            metrics.increment("stream_eof")
            _uiState.value = _uiState.value.copy(lastMessage = "Playlist finished")
            runCatching { ensureRustHostCore().playbackStreamEnded(streamId.value) }
                .onFailure { error -> reportRustHostCommandFailure("finish playlist", error) }
            propagateListenerPlaybackState(
                playbackState = PlaybackState.STOPPED,
                listenerState = _uiState.value.listenerState,
                message = "Host playlist finished",
            )
            runCatching {
                hostTransportController.broadcastStop(
                    streamId = streamId.value,
                    hostStopTimeMs = SystemClock.elapsedRealtime(),
                ).toSendAllResult()
            }.onFailure { error -> handleHostControlFailure("broadcast stream stop", error) }
            refreshHostDiagnostics(streamState = PlaybackState.STOPPED)
            releaseStagedHostSources()
        } catch (error: CancellationException) {
            throw error
        } catch (error: Throwable) {
            lastPacket?.let { sentPacket ->
                awaitHostPacketPresentationTail(
                    sentPacket.hostPresentationTimeMs,
                    presentationOffsetMs,
                    packetDurationMs,
                )
            }
            handleHostPlaybackEngineFailure(error)
        } finally {
            cancelHostBoundaryJobs()
            if (hostPlaylistStreamer === streamer) {
                clearHostPlaylistStream()
            }
        }
    }
}

internal suspend fun awaitHostPacketPresentationTail(
    packetPresentationTimeMs: Long,
    presentationOffsetMs: Long,
    packetDurationMs: Long,
    elapsedRealtimeMs: () -> Long = SystemClock::elapsedRealtime,
    wait: suspend (Long) -> Unit = { delay(it) },
): Long {
    val audibleEndElapsedMs = packetPresentationTimeMs + presentationOffsetMs + packetDurationMs
    val remainingMs = (audibleEndElapsedMs - elapsedRealtimeMs()).coerceAtLeast(0L)
    if (remainingMs > 0L) wait(remainingMs)
    return audibleEndElapsedMs
}

internal fun MainViewModel.handleHostPlaybackEngineFailure(error: Throwable) {
    val message = error.message ?: "Host playback engine failed"
    logger.e("playback.host", message, error)
    hostStreamJob?.cancel()
    stopHostMonitorPlayback()
    _uiState.value = _uiState.value.copy(lastError = message)
    reportRustHostPlaybackFailure(message)
    currentStreamId?.let { failedStreamId ->
        viewModelScope.launch(Dispatchers.IO) {
            runCatching {
                hostTransportController.broadcastStop(
                    streamId = failedStreamId.value,
                    hostStopTimeMs = SystemClock.elapsedRealtime(),
                )
            }.onFailure { stopError ->
                logger.w("stream.stop.failure", stopError.message ?: "Could not stop listeners")
            }
        }
    }
    diagnosticsStore.updateHost {
        it.copy(streamState = PlaybackState.ERROR, lastError = message, metricsSummary = summarizeMetrics())
    }
    refreshHostDiagnostics(streamState = PlaybackState.ERROR)
}

private suspend fun MainViewModel.broadcastHostTrackBoundary(
    streamId: StreamId,
    boundary: FfiPlaylistFrame.TrackBoundary,
) {
    metrics.increment("track_boundary_broadcast")
    runCatching {
        hostTransportController.broadcastTrackBoundary(
            streamId = streamId.value,
            firstSampleIndex = boundary.firstSampleIndex.toLong(),
            entryId = boundary.entryId,
            displayName = boundary.displayName,
        )
    }.onSuccess { delivery ->
        reportHostBroadcastDelivery("broadcast track boundary", delivery.toSendAllResult(), requireAnyPeer = false)
    }.onFailure { error -> handleHostControlFailure("broadcast track boundary", error) }
}

internal fun MainViewModel.scheduleHostQueueBoundary(
    start: FfiPlaylistStreamStart,
    boundary: FfiPlaylistFrame.TrackBoundary,
    sampleRate: Int,
    dispatcher: CoroutineDispatcher = Dispatchers.IO,
    elapsedRealtimeMs: () -> Long = SystemClock::elapsedRealtime,
    wait: suspend (Long) -> Unit = { delay(min(it, PAUSE_POLL_INTERVAL_MS)) },
) {
    metrics.increment("track_boundary_scheduled")
    val job = viewModelScope.launch(dispatcher) {
        val dueAtMs = start.hostStartTimeMs.toLong() +
            boundary.firstSampleIndex.toLong() * 1_000L / sampleRate
        awaitHostQueueBoundary(
            dueAtElapsedRealtimeMs = dueAtMs,
            isPaused = { _uiState.value.hostPlaybackState == PlaybackState.PAUSED },
            elapsedRealtimeMs = elapsedRealtimeMs,
            wait = wait,
        )
        if (!isActive) return@launch
        runCatching {
            ensureRustHostCore().trackBoundaryApplied(boundary.entryId, boundary.firstSampleIndex.toLong())
        }.onSuccess {
            hostPendingTrackBoundaries.remove(boundary.firstSampleIndex.toLong())
            hostPlaylistStreamStart = hostPlaylistStreamStart?.copy(
                currentEntryId = boundary.entryId,
                currentDisplayName = boundary.displayName,
            )
            metrics.increment("track_boundary_applied")
            diagnosticsStore.updateHost { it.copy(metricsSummary = summarizeMetrics()) }
            refreshHostDiagnostics()
        }.onFailure { error -> reportRustHostCommandFailure("advance playlist", error) }
    }
    hostBoundaryJobs += job
    job.invokeOnCompletion { hostBoundaryJobs.remove(job) }
}

internal suspend fun awaitHostQueueBoundary(
    dueAtElapsedRealtimeMs: Long,
    isPaused: () -> Boolean,
    elapsedRealtimeMs: () -> Long = SystemClock::elapsedRealtime,
    wait: suspend (Long) -> Unit = { delay(min(it, PAUSE_POLL_INTERVAL_MS)) },
): Long {
    var dueAtMs = dueAtElapsedRealtimeMs
    var pausedAtMs: Long? = null
    while (true) {
        val now = elapsedRealtimeMs()
        if (isPaused()) {
            if (pausedAtMs == null) pausedAtMs = now
            wait(PAUSE_POLL_INTERVAL_MS)
            continue
        }
        pausedAtMs?.let { dueAtMs += now - it; pausedAtMs = null }
        val remainingMs = dueAtMs - now
        if (remainingMs <= 0L) return dueAtMs
        wait(min(remainingMs, PAUSE_POLL_INTERVAL_MS))
    }
}

internal fun MainViewModel.cancelHostBoundaryJobs() {
    val pendingJobs = hostBoundaryJobs.toList()
    hostBoundaryJobs.clear()
    pendingJobs.forEach(Job::cancel)
}

private const val PAUSE_POLL_INTERVAL_MS = 20L
private const val POSITION_REPORT_INTERVAL_MS = 250L
