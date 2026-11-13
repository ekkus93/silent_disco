package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.app.controller.ListenerStreamStart
import com.ekkus.silentdisco.app.controller.ListenerStreamStartResult
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.uniffi.FfiAudioPacket
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiPlaybackPhase
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay

/** How long a stream may sit without a clock lock before it is reported. */
private const val LISTENER_SYNC_LOCK_TIMEOUT_MS = 5_000L

/** How often the runtime's own accounting is mirrored into the UI. */
private const val LISTENER_DIAGNOSTICS_POLL_MS = 100L

internal fun MainViewModel.handleTransportAudioReceived(event: FfiListenerTransportEvent.AudioReceived) {
    val session = _uiState.value.selectedSession ?: return
    val runtime = listenerPlayback ?: return
    // Ordering, concealment, and scheduling belong to the runtime; a packet it
    // rejects is ordinary traffic its own counters already account for.
    runCatching {
        runtime.submitPacket(
            FfiAudioPacket(
                sequence = event.sequence,
                sampleRate = event.sampleRate,
                channels = event.channels,
                samplesPerPacket = event.samplesPerPacket,
                firstSampleIndex = event.firstSampleIndex,
                hostPresentationTimeMs = event.hostPresentationTimeMs,
                payload = event.payload,
            ),
            session.id,
            event.streamId,
        )
    }.onFailure { error -> handleListenerPlaybackEngineFailure(error) }
}

/**
 * Starts Rust-owned playback of the stream just announced by
 * [FfiListenerTransportEvent.StreamStarted].
 *
 * Everything between an arriving packet and the render ring — ordering,
 * concealment, presentation-time pacing, clock estimation, PCM conversion —
 * belongs to the runtime. This function only opens it, points the native
 * output at its ring, and reflects its state into the UI.
 */
internal fun MainViewModel.startListenerPlaybackFromTransport(
    sessionId: SessionId,
    streamId: StreamId,
    event: FfiListenerTransportEvent.StreamStarted,
    resumeInPlace: Boolean,
) {
    val start = ListenerStreamStart(
        sessionId = sessionId,
        streamId = streamId,
        sampleRate = event.sampleRate,
        hostStartTimeMs = event.hostStartTimeMs,
        samplesPerPacket = event.samplesPerPacket,
        channels = event.channels,
        startupBufferMs = currentPlaybackThresholds().startupBufferMs,
    )
    listenerPlaybackController.execute {
        val result = listenerPlaybackController.startNow(
            start,
            reanchorRunning = shouldReanchorListenerPlayback(resumeInPlace, runtimeAvailable = true),
        )
        result.stopFailures.forEach(::reportListenerPlaybackStopFailure)
        when (result) {
            is ListenerStreamStartResult.Reanchored -> result.failure?.let(::handleListenerPlaybackEngineFailure)
            is ListenerStreamStartResult.Failed -> handleListenerPlaybackEngineFailure(result.error)
            is ListenerStreamStartResult.Started -> {
                _uiState.value = _uiState.value.copy(
                    listenerState = ListenerLifecycleState.BUFFERING,
                    listenerPlaybackState = PlaybackState.BUFFERING,
                )
                startListenerPlaybackDiagnostics(result.runtime)
                // The runtime gates playback on an accepted clock-sync sample, so it
                // needs probes from the moment it exists. This path never started any:
                // every packet was dropped "before lock", and the host rightly evicted a
                // listener that never probed (Block 52.6).
                startPeriodicListenerResync()
            }
        }
    }
}

/**
 * Promotes BUFFERING whenever the runtime plays -- including after a
 * same-stream resume re-anchors it -- but never overrides PAUSED: the runtime
 * keeps playing out its buffered tail after a host pause.
 */
internal fun shouldReportListenerPlaying(phase: FfiPlaybackPhase, reported: PlaybackState): Boolean =
    phase == FfiPlaybackPhase.PLAYING && reported == PlaybackState.BUFFERING

/**
 * Mirrors the runtime's own accounting into the diagnostics store and UI.
 *
 * Polling a snapshot beats deriving state per frame: the counters come from
 * the component that owns the behaviour they describe, so they cannot drift
 * from what actually happened.
 */
private fun MainViewModel.startListenerPlaybackDiagnostics(runtime: FfiListenerPlaybackHandle) {
    listenerPlaybackController.launchStatusPoll(Dispatchers.IO) {
        var reportedSyncStall = false
        var lastUnderruns = 0UL
        var lastDroppedBeforeSync = 0UL
        val openedAtMs = android.os.SystemClock.elapsedRealtime()
        while (_uiState.value.listenerState != ListenerLifecycleState.DISCONNECTED) {
            if (wifiDirectService.snapshot.value.state == TransportConnectionState.DISCONNECTED ||
                wifiDirectService.snapshot.value.state == TransportConnectionState.FAILED
            ) {
                handleListenerDisconnect("Transport disconnected during playback")
                return@launchStatusPoll
            }
            if (listenerPlayback !== runtime) return@launchStatusPoll
            val playback = runtime.diagnostics()
            // A stream that cannot play must never read as playing. Without a
            // real clock offset the pump releases nothing at all, and
            // reporting PLAYING there turns a hard failure into an invisible
            // one -- which is how this path shipped silent.
            if (!playback.syncLocked &&
                android.os.SystemClock.elapsedRealtime() - openedAtMs > LISTENER_SYNC_LOCK_TIMEOUT_MS &&
                !reportedSyncStall
            ) {
                reportedSyncStall = true
                val message = "Couldn't establish a steady connection. Move closer to the host and try again."
                logger.w("sync.stalled", message)
                _uiState.value = _uiState.value.copy(
                    listenerState = ListenerLifecycleState.DESYNCED,
                    lastError = message,
                )
                diagnostics.updateListener { it.copy(lastError = message) }
            }
            if (!playback.syncLocked && playback.droppedBeforeSync > lastDroppedBeforeSync) {
                val message = "Getting your connection ready…"
                logger.w("sync.pre_sync_drop", message)
                _uiState.value = _uiState.value.copy(
                    listenerState = ListenerLifecycleState.SYNCING_CLOCK,
                    lastMessage = message,
                )
            }
            lastDroppedBeforeSync = playback.droppedBeforeSync
            if (shouldReportListenerPlaying(playback.phase, _uiState.value.listenerPlaybackState)) {
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
            }
            if (playback.ringUnderruns > lastUnderruns) {
                logger.w("playback.underrun", "Render ring underruns=${playback.ringUnderruns}")
                lastUnderruns = playback.ringUnderruns
            }
            diagnostics.updateListener {
                it.copy(
                    playbackState = playback.toPlaybackState(),
                    bufferDepthMs = playback.bufferedSpanMs.toLong(),
                    packetLossCount = playback.sequencesSkipped.toInt(),
                    lateDropCount = playback.lateRejections.toInt(),
                    underrunCount = playback.ringUnderruns.toInt(),
                    concealedPacketCount = playback.concealedPackets.toInt(),
                    metricsSummary = summarizeMetrics(),
                )
            }
            refreshListenerDiagnostics()
            delay(LISTENER_DIAGNOSTICS_POLL_MS)
        }
    }
}

/**
 * Stops Rust playback and the native output for the discovered-session path.
 * With [cutAtHostMs], buffered audio presented after that host time is
 * dropped instead of played out (see `FfiListenerPlaybackHandle.stopAt`).
 */
internal fun MainViewModel.stopListenerPlayback(cutAtHostMs: ULong? = null) {
    listenerPlaybackController.cancelStatusPoll()
    listenerPlaybackController.execute {
        listenerPlaybackController.stopNow(cutAtHostMs)?.let(::reportListenerPlaybackStopFailure)
    }
}

private fun MainViewModel.reportListenerPlaybackStopFailure(error: Throwable) {
    logger.w("playback.stop_failed", error.message ?: "playback failed to stop cleanly")
    _uiState.value = _uiState.value.copy(
        listenerPlaybackState = PlaybackState.ERROR,
        lastError = error.message ?: "playback failed to stop cleanly",
    )
}
