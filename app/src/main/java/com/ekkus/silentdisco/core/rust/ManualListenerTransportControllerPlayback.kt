package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.audio.OboeBridge
import com.ekkus.silentdisco.core.model.ManualConnectUiState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.uniffi.FfiAudioPacket
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackConfig
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportHandle
import java.io.File
import kotlinx.coroutines.CoroutineScope

/**
 * Render ring geometry and pacing handed to the Rust playback runtime, which
 * owns every decision made with them. One second of capacity with a 400ms
 * cushion: deep enough that the real-time callback survives writer jitter,
 * shallow enough that playback latency stays bounded.
 */
private const val RING_CAPACITY_FRAMES: UInt = 48_000u
private const val RING_TARGET_FILL_FRAMES: UInt = 19_200u
private const val RING_WRITE_LEAD_MS: ULong = 400uL
private const val MAX_RING_PREFILL_MS: ULong = 800uL

/** `nativeOboeOpen` success status. */
private const val OBOE_ADAPTER_STATUS_OK = 0

/**
 * How much scheduled-timeline span must be buffered before playback starts
 * (forwarded to the Rust scheduler).
 *
 * Was raised to 1000ms for a session as an experiment against observed
 * startup-transient underruns, on the theory that a bigger cushion gives
 * the render ring and coroutine dispatcher more real wall-clock time to
 * reach steady cadence before playback begins. That was before the A1-A3
 * and A6 fixes to the underlying supply bugs (blocking sends, premature
 * peer disconnects, host inbound-silence blindness). Re-measured 2026-08-10
 * (A4.2) with those fixes in place, controlled same-session A/B, 4 real
 * device runs per value: 400ms measurably *beat* 1000ms on every metric
 * that differed, non-overlapping ranges --
 * `ringSilenceFilled` 53,520-62,832 (400ms) vs. 99,504-121,296 (1000ms);
 * `ringUnderruns` 280-328 vs. 519-632; `ringFullEvents` zero in all four
 * 400ms runs vs. nonzero (and the ring hitting its absolute capacity) in
 * two of four 1000ms runs. The original problem this 1000ms experiment
 * targeted no longer reproduces now that the supply side is healthy, and
 * the larger cushion itself was creating a *different* failure mode (ring
 * overflow) that a smaller one doesn't. See `docs/AUDIO_PLAYBACK_STATE_2026-08-10.md`
 * A4.2 for the full numbers.
 */
private const val STARTUP_BUFFER_MS = 400L

/**
 * Span rebuilt before playback resumes after a *mid-stream* rebuffer.
 *
 * Left equal to [STARTUP_BUFFER_MS], which the A4.2 measurement above
 * exercised too (this knob and the startup one were tied for that test,
 * both at 400ms) -- so the same controlled real-device evidence covers
 * this value as well (A4.3), reversing an earlier single-run finding that
 * lowering it to 400ms measured worse. That earlier attempt predates the
 * A1-A3/A6 supply fixes and was never confirmed with repeated runs; this
 * one was. The knob remains separately settable -- the scheduler still
 * supports diverging it from the startup target -- but there is no
 * evidence yet that diverging it from [STARTUP_BUFFER_MS] helps, so it
 * stays tied until there is.
 */
private const val REBUFFER_TARGET_MS = STARTUP_BUFFER_MS

/**
 * Starts Rust-owned playback for one stream.
 *
 * Unlike the previous implementation this does not defer on sync: the
 * runtime refuses to play against a placeholder offset itself, so the
 * stream can be opened as soon as it is announced and will begin the
 * moment a sample is accepted.
 */
internal fun ManualListenerTransportController.handleStreamStarted(
    scope: CoroutineScope,
    handle: FfiListenerTransportHandle,
    event: FfiListenerTransportEvent.StreamStarted,
) {
    val session = sessionId ?: return

    val runningRuntime = playbackRuntime
    if (runningRuntime != null && currentStreamId == StreamId(event.streamId)) {
        // The host re-broadcasts StreamStart for the *same* stream on
        // resume, carrying a presentation-time anchor shifted forward by
        // however long the pause lasted (see the desktop host's
        // pause/resume offset accounting) -- it is not announcing a new
        // stream. Re-anchoring the already-running scheduler in place
        // keeps the render ring, Oboe stream, and receive/sequence
        // counters exactly as they were; tearing down and reopening the
        // engine here would trade the timeline bug this fixes for a
        // guaranteed audible restart on every single resume.
        runCatching {
            runningRuntime.reanchorPresentationTime(event.hostStartTimeMs)
        }.onFailure { error -> handlePlaybackEngineFailure(error) }
        logger.i(
            "manual.audio.stream_reanchored",
            "streamId=${event.streamId} hostStartMs=${event.hostStartTimeMs}",
        )
        return
    }

    // A genuinely new stream. End whatever was playing but keep the
    // native output, so the rebind below reuses the stream this session
    // was granted rather than reopening one. Usually a no-op here, since
    // the host's `Stop` for the previous track has already ended it the
    // same way -- this covers a new stream arriving without one.
    if (endStream(keepOutputOpen = true) != null) return
    receivedCount = 0
    lastReceivedSequence = null

    val runtime = runCatching {
        FfiListenerPlaybackHandle.open(
            FfiListenerPlaybackConfig(
                sessionId = session.value,
                streamId = event.streamId,
                sampleRate = event.sampleRate,
                hostStartTimeMs = event.hostStartTimeMs,
                samplesPerPacket = event.samplesPerPacket,
                channels = event.channels,
                startupBufferTargetMs = STARTUP_BUFFER_MS.toULong(),
                rebufferTargetMs = REBUFFER_TARGET_MS.toULong(),
                ringCapacityFrames = RING_CAPACITY_FRAMES,
                ringTargetFillFrames = RING_TARGET_FILL_FRAMES,
                writeLeadMs = RING_WRITE_LEAD_MS,
                maxPrefillMs = MAX_RING_PREFILL_MS,
                volume = 1.0f,
            ),
        )
    }.getOrElse { error ->
        handlePlaybackEngineFailure(error)
        return
    }
    playbackRuntime = runtime
    currentStreamId = StreamId(event.streamId)
    // Read back-to-back with the runtime's own clock (just constructed,
    // so its `nowMs()` is ~0) so `transportClockOriginMs` is the
    // transport clock's reading at the runtime's own t=0. See
    // `translateToPumpClock` for how this turns a transport-clock
    // receive timestamp into the runtime's timeline.
    transportClockOriginMs = runCatching { handle.nowMs() }.getOrNull()

    // Reuse the stream this session was already granted whenever one is
    // open, so a track change swaps only the content. Reopening is the
    // fallback, not the default: a reopened stream is what the device
    // downgraded from Exclusive to Shared. A rebind that reports
    // anything other than success falls back to a fresh open rather
    // than leaving the output bound to a stream that is gone.
    val engineToken = runtime.engineToken()
    var oboeStatus = if (OboeBridge.nativeOboeIsOpen()) {
        OboeBridge.nativeOboeRebind(engineToken)
    } else {
        OboeBridge.nativeOboeOpen(engineToken)
    }
    if (oboeStatus != OBOE_ADAPTER_STATUS_OK) {
        OboeBridge.nativeOboeClose()
        oboeStatus = OboeBridge.nativeOboeOpen(engineToken)
    }
    if (oboeStatus != OBOE_ADAPTER_STATUS_OK) {
        stopPlayback()
        handlePlaybackEngineFailure(IllegalStateException("Oboe stream failed to open (status=$oboeStatus)"))
        return
    }
    // Hand the audio path to Rust: from here the transport submits each
    // received datagram straight into this runtime, so audio no longer
    // crosses the binding at all. Before this, every packet surfaced as
    // its own event on the single event-loop coroutine below and was
    // copied out of Rust and immediately back in -- 200 round trips a
    // second, which delayed the control traffic queued behind it (sync
    // responses were timestamped ~140ms late against a 7.7ms real round
    // trip) and is what drove the rebuffering heard as dropouts.
    handle.attachPlayback(runtime)
    startDebugCapture(runtime, event.streamId)
    startDiagnosticsSampler(scope, runtime)
    logger.i(
        "manual.audio.stream_started",
        "streamId=${event.streamId} hostStartMs=${event.hostStartTimeMs} " +
            "sampleRate=${event.sampleRate} samplesPerPacket=${event.samplesPerPacket}",
    )
    _connectState.value = ManualConnectUiState.Streaming(trustedForFuture, PlaybackState.BUFFERING)
    startSyncProbeLoop(scope, handle)
}

/**
 * Enables the debug PCM capture when a recording directory is configured.
 *
 * Diagnostic instrumentation for this viability PoC: the capture is what
 * makes audio defects objectively measurable rather than a matter of
 * describing what playback sounded like.
 */
internal fun ManualListenerTransportController.startDebugCapture(runtime: FfiListenerPlaybackHandle, streamId: String) {
    val directory = debugRecordingDirectory ?: return
    val file = File(directory, "manual-listener-$streamId.wav")
    runCatching { runtime.startDebugCapture(file.absolutePath) }
        .onSuccess { logger.i("manual.audio.recording_started", file.absolutePath) }
        .onFailure { error ->
            logger.w("manual.audio.recording_failed", error.message ?: "debug capture failed to start")
        }
}

internal fun ManualListenerTransportController.handleStreamStopped() {
    // The runtime drains its own buffered tail as part of stopping, so a
    // stream's final moments are played rather than discarded. The native
    // output deliberately stays open: the session is still live and the
    // host may start another track, which then rebinds this same stream
    // instead of reopening one (see [endStream]).
    if (endStream(keepOutputOpen = true) == null) {
        _connectState.value = ManualConnectUiState.Approved(trustedForFuture)
    }
}

/**
 * Fully tears down playback *and* the native output. For ending the
 * current stream while the session continues, use
 * `endStream(keepOutputOpen = true)` instead.
 */
internal fun ManualListenerTransportController.stopPlayback(): Throwable? = endStream(keepOutputOpen = false)

/**
 * Ends the current stream, logging its final accounting.
 *
 * `keepOutputOpen` decides whether the native Oboe stream survives. The
 * output device belongs to the *connection*, not to one track: a device
 * that grants an Exclusive/low-latency stream on the first open may grant
 * only `Shared` when that stream is closed and immediately reopened
 * (observed on a real Android 8.0 device), which makes a second track
 * play through a measurably different output path than the first. So a
 * track change keeps the stream and rebinds it, and only a genuine
 * teardown -- disconnect, rejection, or closing the controller -- closes
 * it.
 */
internal fun ManualListenerTransportController.endStream(keepOutputOpen: Boolean): Throwable? {
    syncProbeJob?.cancel()
    syncProbeJob = null
    diagnosticsSampleJob?.cancel()
    diagnosticsSampleJob = null
    var firstFailure: Throwable? = null
    fun capture(block: () -> Unit) {
        try {
            block()
        } catch (error: Throwable) {
            firstFailure = mergeManualCleanupFailure(firstFailure, error)
        }
    }

    val runtime = playbackRuntime ?: run {
        // No stream to end, but a teardown still has to release the
        // output; leaving a low-latency stream running against a
        // released token would burn the radio and the CPU for silence.
        if (!keepOutputOpen) capture { OboeBridge.nativeOboeClose() }
        firstFailure?.let { error ->
            _connectState.value = ManualConnectUiState.Failed(
                error.message ?: "playback teardown failed",
            )
        }
        return firstFailure
    }
    playbackRuntime = null
    currentStreamId = null
    transportClockOriginMs = null
    // Detach before stopping: the transport must not submit into a
    // runtime that is shutting down. Every cleanup step is still attempted
    // if one fails; the first failure remains primary and later failures are
    // attached as suppressed exceptions.
    capture { handleRef.get()?.detachPlayback() }
    // Order matters and is deliberate: `stop()` drains the render ring
    // *through the still-running Oboe callback* so the stream ends on its
    // own final sample rather than mid-note (see `await_ring_drain`, which
    // documents that a consumer closed first means the ring never drains).
    capture { runtime.stop() }
    if (!keepOutputOpen) capture { OboeBridge.nativeOboeClose() }
    capture { logPlaybackSummary(runtime) }
    capture { runtime.close() }

    firstFailure?.let { error ->
        logger.w("manual.audio.teardown_failed", error.message ?: "playback teardown failed")
        _connectState.value = ManualConnectUiState.Failed(
            error.message ?: "playback teardown failed",
        )
    }
    return firstFailure
}

internal fun ManualListenerTransportController.handleAudioReceived(event: FfiListenerTransportEvent.AudioReceived) {
    val session = sessionId ?: return
    val runtime = playbackRuntime ?: return
    receivedCount += 1
    val sequence = event.sequence.toLong()
    val previousSequence = lastReceivedSequence
    if (previousSequence != null && sequence != previousSequence + 1) {
        logger.w(
            "manual.audio.received_gap",
            "expected seq ${previousSequence + 1} but received $sequence " +
                "(network loss or reorder before the scheduler ever sees it)",
        )
    }
    lastReceivedSequence = sequence
    // Ordering, concealment, and scheduling all belong to the runtime; a
    // packet it rejects (duplicate, late, out of window) is ordinary
    // traffic that its own counters already account for.
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
            session.value,
            event.streamId,
        )
    }.onFailure { error -> handlePlaybackEngineFailure(error) }
}

internal fun ManualListenerTransportController.handlePlaybackEngineFailure(error: Throwable) {
    stopPlayback()?.let(error::addSuppressed)
    _connectState.value = ManualConnectUiState.Failed(error.message ?: "playback engine failed")
}
