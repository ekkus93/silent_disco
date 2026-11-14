package com.ekkus.silentdisco.app.controller

import com.ekkus.silentdisco.app.HostQueueItem
import com.ekkus.silentdisco.app.PendingHostTrackBoundary
import com.ekkus.silentdisco.app.awaitHostPacketPresentationTail
import com.ekkus.silentdisco.app.toSendAllResult
import com.ekkus.silentdisco.core.audio.AudioFormatSpec
import com.ekkus.silentdisco.core.logging.AppLogger
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.rust.HostCoreController
import com.ekkus.silentdisco.core.rust.HostPlaylistStreamController
import com.ekkus.silentdisco.core.rust.HostTransportController
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistFrame
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistStreamStart
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.withContext

private const val PAUSE_POLL_INTERVAL_MS = 20L
private const val POSITION_REPORT_INTERVAL_MS = 250L

/** What the stream loop reads from, and reports to, the coordinator and the host session. */
internal interface HostStreamEnvironment {
    /** The host playback state Rust last accepted. */
    val playbackState: PlaybackState

    /** The queue entry playing now, as the UI lists it. */
    val queueHead: HostQueueItem?

    /** The host core. */
    fun core(): HostCoreController

    suspend fun broadcastStreamStart(streamId: StreamId, start: FfiPlaylistStreamStart)

    /** Sends a boundary to every listener; throws when any connected listener missed it. */
    suspend fun broadcastTrackBoundary(streamId: StreamId, boundary: FfiPlaylistFrame.TrackBoundary)

    /** Starts the timer that advances the host queue at [boundary]. */
    fun scheduleBoundary(start: FfiPlaylistStreamStart, boundary: FfiPlaylistFrame.TrackBoundary, sampleRate: Int)

    fun engineFailed(error: Throwable)
    fun reportPlaybackFailure(message: String)
    fun reportCommandFailure(action: String, error: Throwable)
    fun controlFailed(action: String, error: Throwable)
    fun showError(message: String)
    fun showMessage(message: String)
    fun refreshDiagnostics(streamState: PlaybackState = playbackState)

    /** The playlist ended: tell the listener-facing UI the host stopped. */
    fun reportPlaylistFinished()
}

/**
 * The host stream loop (God-file refactor spec §4, HostPlayback): pulls
 * frames from the Rust streamer and sends each packet to the local monitor
 * and every listener at its presentation time, holds across pauses and
 * re-anchors on resume, forwards track boundaries, and ends the stream after
 * the last packet has played out.
 *
 * Host timing state (spec A.4 T1/T3): the pause offset and packet pacing
 * stay in Kotlin, logged for a later move into Rust.
 */
internal class HostStreamLoop(
    private val playback: HostPlaybackController,
    private val monitor: HostMonitorController,
    private val transport: HostTransportController,
    private val diagnostics: DiagnosticsController,
    private val logger: AppLogger,
    private val clock: () -> Long,
    private val dispatcher: CoroutineDispatcher,
    private val env: HostStreamEnvironment,
) {
    suspend fun run(
        streamId: StreamId,
        streamer: HostPlaylistStreamController,
        firstPacket: AudioPacket,
        format: AudioFormatSpec,
    ) {
        var packet = firstPacket
        var pendingFirstPacket: AudioPacket? = firstPacket
        var packetCount = 0L
        var consecutiveAudioSendFailures = 0
        var presentationOffsetMs = 0L
        var lastReportedPositionMs = -POSITION_REPORT_INTERVAL_MS
        var pauseStartedAtMs: Long? = null
        var previousSendElapsedMs: Long? = null
        var lastPacket: AudioPacket? = null
        val start = playback.streamStart ?: streamer.streamStart
        val packetDurationMs = firstPacket.samplesPerPacket * 1_000L / firstPacket.sampleRate
        var authoritativePlaybackObserved = false

        try {
            while (currentCoroutineContext().isActive) {
                while (!authoritativePlaybackObserved) {
                    when (env.playbackState) {
                        PlaybackState.PLAYING, PlaybackState.PAUSED -> authoritativePlaybackObserved = true
                        PlaybackState.ERROR -> return
                        PlaybackState.STOPPED,
                        PlaybackState.BUFFERING,
                        PlaybackState.READY,
                        PlaybackState.UNDERRUN,
                        -> delay(PAUSE_POLL_INTERVAL_MS)
                    }
                }
                while (env.playbackState == PlaybackState.PAUSED) {
                    if (pauseStartedAtMs == null) pauseStartedAtMs = clock()
                    delay(PAUSE_POLL_INTERVAL_MS)
                }
                pauseStartedAtMs?.let { pausedAt ->
                    presentationOffsetMs += clock() - pausedAt
                    pauseStartedAtMs = null
                    previousSendElapsedMs = null
                    runCatching {
                        monitor.reanchor(
                            (lastPacket?.hostPresentationTimeMs ?: firstPacket.hostPresentationTimeMs) +
                                presentationOffsetMs,
                        )
                    }.onFailure { error ->
                        env.engineFailed(error)
                        return
                    }
                    val resumedStart = start.copy(
                        hostStartTimeMs = start.hostStartTimeMs + presentationOffsetMs.toULong(),
                        currentEntryId = env.queueHead?.entryId,
                        currentDisplayName = env.queueHead?.displayName,
                    )
                    playback.streamStart = resumedStart
                    env.broadcastStreamStart(streamId, resumedStart)
                    playback.pendingBoundaries.values
                        .sortedBy(PendingHostTrackBoundary::firstSampleIndex)
                        .forEach { pending ->
                            env.broadcastTrackBoundary(
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
                if (env.playbackState in setOf(PlaybackState.STOPPED, PlaybackState.ERROR)) {
                    return
                }

                val prefetchedPacket = pendingFirstPacket
                if (prefetchedPacket != null) {
                    packet = prefetchedPacket
                    pendingFirstPacket = null
                } else {
                    when (val frame = withContext(dispatcher) { streamer.nextFrame(3uL) }) {
                        null -> continue
                        FfiPlaylistFrame.End -> break
                        is FfiPlaylistFrame.TrackBoundary -> {
                            val actor = env.core()
                            // In production the producer already reserved the
                            // boundary with Rust before queuing successor PCM.
                            if (!actor.boundariesReservedByProducer) {
                                actor.trackBoundaryScheduled(
                                    frame.entryId,
                                    frame.firstSampleIndex.toLong(),
                                )
                            }
                            playback.pendingBoundaries[frame.firstSampleIndex.toLong()] =
                                PendingHostTrackBoundary(
                                    streamId = frame.streamId,
                                    firstSampleIndex = frame.firstSampleIndex.toLong(),
                                    entryId = frame.entryId,
                                    displayName = frame.displayName,
                                )
                            env.broadcastTrackBoundary(streamId, frame)
                            env.scheduleBoundary(
                                // New boundaries must share the pause-adjusted
                                // clock of boundaries scheduled before Resume.
                                start.copy(
                                    hostStartTimeMs = start.hostStartTimeMs +
                                        presentationOffsetMs.toULong(),
                                ),
                                frame,
                                format.sampleRate,
                            )
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
                val now = clock()
                previousSendElapsedMs?.let { previous ->
                    val sendGap = now - previous
                    diagnostics.metrics.recordTiming("packet_gap_ms", sendGap.toDouble())
                    if (kotlin.math.abs(sendGap - packetDurationMs) > 8) {
                        diagnostics.metrics.increment("packet_send_anomaly")
                        logger.w("packet.send.anomaly", "seq=${packet.sequenceNumber} gap=${sendGap}ms")
                    }
                }
                packetCount += 1
                if (packetCount % 25L == 1L) {
                    logger.i("stream.packet", "stream=${streamId.value} sent=$packetCount")
                }
                diagnostics.metrics.increment("packet_send_total")
                diagnostics.updateHost {
                    it.copy(
                        packetSendCount = packetCount,
                        packetSendRatePerSecond = 1_000.0 / packetDurationMs,
                        lastContactElapsedMs = now,
                        metricsSummary = diagnostics.summarizeMetrics(),
                    )
                }
                val playbackPositionMs = timedPacket.firstSampleIndex * 1_000L / timedPacket.sampleRate
                if (playbackPositionMs >= lastReportedPositionMs + POSITION_REPORT_INTERVAL_MS) {
                    runCatching {
                        env.core().playbackPositionAdvanced(streamId.value, playbackPositionMs)
                    }.onFailure { error ->
                        logger.w("playback.position", error.message ?: "Could not report host playback position")
                    }
                    lastReportedPositionMs = playbackPositionMs
                }
                runCatching { monitor.submit(timedPacket) }
                    .onFailure { error ->
                        if (env.playbackState != PlaybackState.STOPPED) {
                            env.engineFailed(error)
                        }
                        return
                    }
                runCatching {
                    transport.broadcastAudio(
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
                            env.showError(message)
                            diagnostics.updateHost { it.copy(lastError = message, metricsSummary = diagnostics.summarizeMetrics()) }
                            env.refreshDiagnostics()
                        }
                        result.failureCount > 0 -> {
                            consecutiveAudioSendFailures += 1
                            val message = "Audio packet delivered to ${result.successCount}/${result.peerCount} listeners; ${result.failureCount} failed"
                            logger.w("transport.audio", message)
                            env.showError(message)
                            diagnostics.updateHost { it.copy(lastError = message, metricsSummary = diagnostics.summarizeMetrics()) }
                            env.refreshDiagnostics()
                        }
                        else -> {
                            consecutiveAudioSendFailures = 0
                            diagnostics.updateHost { it.copy(lastError = null) }
                        }
                    }
                }.onFailure { error ->
                    consecutiveAudioSendFailures += 1
                    val message = error.message ?: "Failed to send audio packet"
                    logger.w("transport.audio", "Failed to send packet ${timedPacket.sequenceNumber}: $message")
                    env.showError(message)
                    diagnostics.updateHost { it.copy(lastError = message, metricsSummary = diagnostics.summarizeMetrics()) }
                    env.refreshDiagnostics()
                }
                if (consecutiveAudioSendFailures >= 10) {
                    val message = "Audio transport failed repeatedly; stream stopped"
                    env.showError(message)
                    env.reportPlaybackFailure(message)
                    diagnostics.updateHost {
                        it.copy(streamState = PlaybackState.ERROR, lastError = message, metricsSummary = diagnostics.summarizeMetrics())
                    }
                    env.refreshDiagnostics(streamState = PlaybackState.ERROR)
                    return
                }
                env.refreshDiagnostics()
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
                    clock,
                )
            }
            monitor.stop()
            logger.i("stream.stop", "Host playlist reached its final track boundary")
            diagnostics.metrics.increment("stream_eof")
            env.showMessage("Playlist finished")
            runCatching { env.core().playbackStreamEnded(streamId.value) }
                .onFailure { error -> env.reportCommandFailure("finish playlist", error) }
            env.reportPlaylistFinished()
            runCatching {
                transport.broadcastStop(
                    streamId = streamId.value,
                    hostStopTimeMs = clock(),
                ).toSendAllResult()
            }.onFailure { error -> env.controlFailed("broadcast stream stop", error) }
            env.refreshDiagnostics(streamState = PlaybackState.STOPPED)
            playback.clearStream()
            playback.staging.releaseAll()
        } catch (error: CancellationException) {
            throw error
        } catch (error: Throwable) {
            // A boundary at or past the end of the sent audio introduces a
            // track no listener will hear. Cancel it before waiting out the
            // tail: its deadline equals the tail's end, and the host queue
            // must not advance to it on whichever timer happens to win.
            playback.cancelBoundaryTimersFrom(
                lastPacket?.let { it.firstSampleIndex + it.samplesPerPacket } ?: 0L,
            )
            lastPacket?.let { sentPacket ->
                awaitHostPacketPresentationTail(
                    sentPacket.hostPresentationTimeMs,
                    presentationOffsetMs,
                    packetDurationMs,
                    clock,
                )
            }
            env.engineFailed(error)
        } finally {
            playback.cancelBoundaryTimers()
            if (playback.streamer === streamer) {
                playback.clearStream()
            }
        }
    }
}
