package com.ekkus.silentdisco.app

import android.os.SystemClock
import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.app.controller.HostStreamEnvironment
import com.ekkus.silentdisco.app.controller.HostStreamLoop
import com.ekkus.silentdisco.core.rust.HostCoreController
import com.ekkus.silentdisco.core.audio.AudioFormatSpec
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.rust.HostPlaylistStreamController
import com.ekkus.silentdisco.core.transport.SendAllResult
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportDelivery
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistFrame
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistStreamStart
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
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
    val loop = HostStreamLoop(
        playback = hostPlayback,
        monitor = hostMonitor,
        transport = hostTransportController,
        diagnostics = diagnostics,
        logger = logger,
        clock = hostElapsedRealtimeMs,
        dispatcher = hostStreamDispatcher,
        env = hostStreamEnvironment(),
    )
    hostPlayback.launchStream(hostStreamDispatcher) { loop.run(streamId, streamer, firstPacket, format) }
}

/** How the stream loop reads the UI state and reaches the host core, transport and failure paths. */
private fun MainViewModel.hostStreamEnvironment() = object : HostStreamEnvironment {
    override val playbackState: PlaybackState get() = _uiState.value.hostPlaybackState
    override val queueHead: HostQueueItem? get() = _uiState.value.hostQueue.firstOrNull()
    override fun core(): HostCoreController = ensureRustHostCore()
    override suspend fun broadcastStreamStart(streamId: StreamId, start: FfiPlaylistStreamStart) =
        broadcastHostStreamStart(streamId, start)
    override suspend fun broadcastTrackBoundary(streamId: StreamId, boundary: FfiPlaylistFrame.TrackBoundary) =
        broadcastHostTrackBoundary(streamId, boundary)
    override fun scheduleBoundary(
        start: FfiPlaylistStreamStart,
        boundary: FfiPlaylistFrame.TrackBoundary,
        sampleRate: Int,
    ) = scheduleHostQueueBoundary(start, boundary, sampleRate, hostStreamDispatcher, hostElapsedRealtimeMs)
    override fun engineFailed(error: Throwable) = handleHostPlaybackEngineFailure(error)
    override fun reportPlaybackFailure(message: String) = reportRustHostPlaybackFailure(message)
    override fun reportCommandFailure(action: String, error: Throwable) = reportRustHostCommandFailure(action, error)
    override fun controlFailed(action: String, error: Throwable) = handleHostControlFailure(action, error)
    override fun showError(message: String) {
        _uiState.value = _uiState.value.copy(lastError = message)
    }
    override fun showMessage(message: String) {
        _uiState.value = _uiState.value.copy(lastMessage = message)
    }
    override fun refreshDiagnostics(streamState: PlaybackState) = refreshHostDiagnostics(streamState = streamState)
    override fun reportPlaylistFinished() = propagateListenerPlaybackState(
        playbackState = PlaybackState.STOPPED,
        listenerState = _uiState.value.listenerState,
        message = "Host playlist finished",
    )
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
    hostPlayback.cancelStream()
    stopHostMonitorPlayback()
    _uiState.value = _uiState.value.copy(lastError = message)
    reportRustHostPlaybackFailure(message)
    hostPlayback.streamId?.let { failedStreamId ->
        viewModelScope.launch(hostStreamDispatcher) {
            runCatching {
                hostTransportController.broadcastStop(
                    streamId = failedStreamId.value,
                    hostStopTimeMs = hostElapsedRealtimeMs(),
                )
            }.onFailure { stopError ->
                logger.w("stream.stop.failure", stopError.message ?: "Could not stop listeners")
            }
        }
    }
    diagnostics.updateHost {
        it.copy(streamState = PlaybackState.ERROR, lastError = message, metricsSummary = summarizeMetrics())
    }
    refreshHostDiagnostics(streamState = PlaybackState.ERROR)
}

private suspend fun MainViewModel.broadcastHostTrackBoundary(
    streamId: StreamId,
    boundary: FfiPlaylistFrame.TrackBoundary,
) {
    diagnostics.metrics.increment("track_boundary_broadcast")
    // Unlike a best-effort UI notification, this control defines which
    // source listeners are about to hear. Sending subsequent PCM after a
    // failed boundary would make the listener's track identity untruthful.
    val delivery = try {
        hostTransportController.broadcastTrackBoundary(
            streamId = streamId.value,
            firstSampleIndex = boundary.firstSampleIndex.toLong(),
            entryId = boundary.entryId,
            displayName = boundary.displayName,
        ).toSendAllResult()
    } catch (error: Throwable) {
        handleHostControlFailure("broadcast track boundary", error)
        throw error
    }
    if (!reportHostBroadcastDelivery("broadcast track boundary", delivery, requireAnyPeer = false)) {
        // The stream loop catches this, withdraws unheard boundaries, waits
        // for the final sent PCM tail, then broadcasts Stop. Zero listeners
        // is permitted: there is no connected listener to desynchronize.
        throw IllegalStateException("Track boundary was not delivered to every connected listener")
    }
}

internal fun MainViewModel.scheduleHostQueueBoundary(
    start: FfiPlaylistStreamStart,
    boundary: FfiPlaylistFrame.TrackBoundary,
    sampleRate: Int,
    dispatcher: CoroutineDispatcher = Dispatchers.IO,
    elapsedRealtimeMs: () -> Long = SystemClock::elapsedRealtime,
    wait: suspend (Long) -> Unit = { delay(min(it, PAUSE_POLL_INTERVAL_MS)) },
) {
    diagnostics.metrics.increment("track_boundary_scheduled")
    hostPlayback.launchBoundaryTimer(boundary.firstSampleIndex.toLong(), dispatcher) {
        val dueAtMs = start.hostStartTimeMs.toLong() +
            boundary.firstSampleIndex.toLong() * 1_000L / sampleRate
        awaitHostQueueBoundary(
            dueAtElapsedRealtimeMs = dueAtMs,
            isPaused = { _uiState.value.hostPlaybackState == PlaybackState.PAUSED },
            elapsedRealtimeMs = elapsedRealtimeMs,
            wait = wait,
        )
        if (!isActive) return@launchBoundaryTimer
        runCatching {
            ensureRustHostCore().trackBoundaryApplied(boundary.entryId, boundary.firstSampleIndex.toLong())
        }.onSuccess {
            hostPlayback.pendingBoundaries.remove(boundary.firstSampleIndex.toLong())
            hostPlayback.streamStart = hostPlayback.streamStart?.copy(
                currentEntryId = boundary.entryId,
                currentDisplayName = boundary.displayName,
            )
            diagnostics.metrics.increment("track_boundary_applied")
            diagnostics.updateHost { it.copy(metricsSummary = summarizeMetrics()) }
            refreshHostDiagnostics()
        }.onFailure { error -> reportRustHostCommandFailure("advance playlist", error) }
    }
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

internal fun MainViewModel.cancelHostBoundaryJobs() = hostPlayback.cancelBoundaryTimers()

private const val PAUSE_POLL_INTERVAL_MS = 20L
