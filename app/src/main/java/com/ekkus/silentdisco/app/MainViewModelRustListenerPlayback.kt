package com.ekkus.silentdisco.app

import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.core.audio.OboeBridge
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.uniffi.FfiAudioPacket
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackConfig
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiPlaybackPhase
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

private const val LISTENER_REBUFFER_TARGET_MS: ULong = 400uL

private const val LISTENER_RING_CAPACITY_FRAMES: UInt = 48_000u
private const val LISTENER_RING_TARGET_FILL_FRAMES: UInt = 19_200u
private const val LISTENER_RING_WRITE_LEAD_MS: ULong = 400uL
private const val LISTENER_MAX_RING_PREFILL_MS: ULong = 800uL

/** How long a stream may sit without a clock lock before it is reported. */
private const val LISTENER_SYNC_LOCK_TIMEOUT_MS = 5_000L

/** How often the runtime's own accounting is mirrored into the UI. */
private const val LISTENER_DIAGNOSTICS_POLL_MS = 100L

/** `nativeOboeOpen` success status. */
private const val LISTENER_OBOE_STATUS_OK = 0

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
    listenerPlaybackControlExecutor.execute {
        startListenerPlaybackFromTransportNow(sessionId, streamId, event, resumeInPlace)
    }
}

private fun MainViewModel.startListenerPlaybackFromTransportNow(
    sessionId: SessionId,
    streamId: StreamId,
    event: FfiListenerTransportEvent.StreamStarted,
    resumeInPlace: Boolean,
) {
    val runningPlayback = listenerPlayback
    if (
        runningPlayback != null &&
        shouldReanchorListenerPlayback(resumeInPlace, runtimeAvailable = true)
    ) {
        runCatching { runningPlayback.reanchorPresentationTime(event.hostStartTimeMs) }
            .onFailure(::handleListenerPlaybackEngineFailure)
        return
    }
    stopListenerPlaybackNow()
    val runtime = runCatching {
        FfiListenerPlaybackHandle.open(
            FfiListenerPlaybackConfig(
                sessionId = sessionId.value,
                streamId = streamId.value,
                sampleRate = event.sampleRate,
                hostStartTimeMs = event.hostStartTimeMs,
                samplesPerPacket = event.samplesPerPacket,
                channels = event.channels,
                startupBufferTargetMs = currentPlaybackThresholds().startupBufferMs.toULong(),
                // Clamped by the scheduler to the startup target above, so a tuning
                // profile with a shallower startup buffer keeps its fast recovery.
                rebufferTargetMs = LISTENER_REBUFFER_TARGET_MS,
                ringCapacityFrames = LISTENER_RING_CAPACITY_FRAMES,
                ringTargetFillFrames = LISTENER_RING_TARGET_FILL_FRAMES,
                writeLeadMs = LISTENER_RING_WRITE_LEAD_MS,
                maxPrefillMs = LISTENER_MAX_RING_PREFILL_MS,
                volume = 1.0f,
            ),
        )
    }.getOrElse { error ->
        handleListenerPlaybackEngineFailure(error)
        return
    }
    listenerPlayback = runtime

    val oboeStatus = OboeBridge.nativeOboeOpen(runtime.engineToken())
    if (oboeStatus != LISTENER_OBOE_STATUS_OK) {
        stopListenerPlaybackNow()
        handleListenerPlaybackEngineFailure(
            IllegalStateException("Oboe stream failed to open (status=$oboeStatus)"),
        )
        return
    }
    try {
        listenerTransportController.attachPlayback(runtime)
    } catch (error: Throwable) {
        stopListenerPlaybackNow()
        handleListenerPlaybackEngineFailure(error)
        return
    }
    _uiState.value = _uiState.value.copy(
        listenerState = ListenerLifecycleState.BUFFERING,
        listenerPlaybackState = PlaybackState.BUFFERING,
    )
    startListenerPlaybackDiagnostics(runtime)
    // The runtime gates playback on an accepted clock-sync sample, so it
    // needs probes from the moment it exists. This path never started any:
    // every packet was dropped "before lock", and the host rightly evicted a
    // listener that never probed (Block 52.6).
    startPeriodicListenerResync()
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
    playbackJob?.cancel()
    playbackJob = viewModelScope.launch(Dispatchers.IO) {
        var reportedSyncStall = false
        var lastUnderruns = 0UL
        var lastDroppedBeforeSync = 0UL
        val openedAtMs = android.os.SystemClock.elapsedRealtime()
        while (_uiState.value.listenerState != ListenerLifecycleState.DISCONNECTED) {
            if (wifiDirectService.snapshot.value.state == TransportConnectionState.DISCONNECTED ||
                wifiDirectService.snapshot.value.state == TransportConnectionState.FAILED
            ) {
                handleListenerDisconnect("Transport disconnected during playback")
                return@launch
            }
            if (listenerPlayback !== runtime) return@launch
            val diagnostics = runtime.diagnostics()
            // A stream that cannot play must never read as playing. Without a
            // real clock offset the pump releases nothing at all, and
            // reporting PLAYING there turns a hard failure into an invisible
            // one -- which is how this path shipped silent.
            if (!diagnostics.syncLocked &&
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
                diagnosticsStore.updateListener { it.copy(lastError = message) }
            }
            if (!diagnostics.syncLocked && diagnostics.droppedBeforeSync > lastDroppedBeforeSync) {
                val message = "Getting your connection ready…"
                logger.w("sync.pre_sync_drop", message)
                _uiState.value = _uiState.value.copy(
                    listenerState = ListenerLifecycleState.SYNCING_CLOCK,
                    lastMessage = message,
                )
            }
            lastDroppedBeforeSync = diagnostics.droppedBeforeSync
            if (shouldReportListenerPlaying(diagnostics.phase, _uiState.value.listenerPlaybackState)) {
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
            if (diagnostics.ringUnderruns > lastUnderruns) {
                logger.w("playback.underrun", "Render ring underruns=${diagnostics.ringUnderruns}")
                lastUnderruns = diagnostics.ringUnderruns
            }
            diagnosticsStore.updateListener {
                it.copy(
                    playbackState = diagnostics.toPlaybackState(),
                    bufferDepthMs = diagnostics.bufferedSpanMs.toLong(),
                    packetLossCount = diagnostics.sequencesSkipped.toInt(),
                    lateDropCount = diagnostics.lateRejections.toInt(),
                    underrunCount = diagnostics.ringUnderruns.toInt(),
                    concealedPacketCount = diagnostics.concealedPackets.toInt(),
                    metricsSummary = summarizeMetrics(),
                )
            }
            refreshListenerDiagnostics()
            delay(LISTENER_DIAGNOSTICS_POLL_MS)
        }
    }
}

/** Stops Rust playback and the native output for the discovered-session path. */
internal fun MainViewModel.stopListenerPlayback() {
    playbackJob?.cancel()
    playbackJob = null
    listenerPlaybackControlExecutor.execute { stopListenerPlaybackNow() }
}

private fun MainViewModel.stopListenerPlaybackNow() {
    val runtime = listenerPlayback ?: return
    listenerPlayback = null
    var firstFailure: Throwable? = null
    fun capture(block: () -> Unit) {
        try {
            block()
        } catch (error: Throwable) {
            val first = firstFailure
            if (first == null) firstFailure = error else first.addSuppressed(error)
        }
    }
    capture { listenerTransportController.detachPlayback() }
    capture { runtime.stop() }
    capture { OboeBridge.nativeOboeClose() }
    capture { runtime.close() }
    firstFailure?.let { error ->
        logger.w("playback.stop_failed", error.message ?: "playback failed to stop cleanly")
        _uiState.value = _uiState.value.copy(
            listenerPlaybackState = PlaybackState.ERROR,
            lastError = error.message ?: "playback failed to stop cleanly",
        )
    }
}
