package com.ekkus.silentdisco.core.rust

import android.os.SystemClock
import com.ekkus.silentdisco.core.audio.AudioFormatSpec
import com.ekkus.silentdisco.core.audio.OboeBridge
import com.ekkus.silentdisco.core.audio.OboePlaybackEngine
import com.ekkus.silentdisco.core.audio.PlaybackEngine
import com.ekkus.silentdisco.core.audio.PlaybackFrame
import com.ekkus.silentdisco.core.audio.PlaybackThresholds
import com.ekkus.silentdisco.core.logging.AppLogger
import com.ekkus.silentdisco.core.model.ManualConnectUiState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.model.SyncState
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.protocol.SyncResponsePacket
import com.ekkus.silentdisco.core.sync.HostTimeMapper
import com.ekkus.silentdisco.core.sync.ListenerSyncController
import com.ekkus.silentdisco.core.sync.SyncMaintenanceConfig
import com.ekkus.silentdisco.core.uniffi.FfiAudioPacket
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackConfig
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportException
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportHandle
import com.ekkus.silentdisco.core.uniffi.FfiManualHostEndpoint
import com.ekkus.silentdisco.core.uniffi.parseManualHostEndpoint
import java.io.File
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Result of validating a pasted/typed manual connection payload without connecting. */
sealed interface ManualEndpointParseResult {
    data class Valid(val endpoint: FfiManualHostEndpoint) : ManualEndpointParseResult

    data class Invalid(val message: String) : ManualEndpointParseResult
}

private const val LOCAL_BIND_ADDRESS = "0.0.0.0"
private const val POLL_TIMEOUT_MS: ULong = 500uL
private const val AUDIO_CODEC_NAME = "pcm16le"

/** Durable listener-side diagnostics, beside the debug PCM captures. */
private const val DIAGNOSTICS_LOG_FILE_NAME = "manual-listener-diagnostics.log"

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

/**
 * Interval between clock-sync probes once the clock is locked.
 *
 * Before it locks, probes go out at [SYNC_PROBE_ACQUIRE_CADENCE_MS] instead:
 * playback produces nothing at all until a sample is accepted, and the
 * estimator rejects any sample whose round trip exceeds its bound, so a
 * steady 2s cadence turns three unlucky probes into six seconds of silence.
 */
private const val DIAGNOSTICS_SAMPLE_CADENCE_MS = 1_000L
private const val SYNC_PROBE_CADENCE_MS = 2_000L
private const val SYNC_PROBE_ACQUIRE_CADENCE_MS = 250L

/** `nativeOboeOpen` success status. */
private const val OBOE_ADAPTER_STATUS_OK = 0

/**
 * How much scheduled-timeline span must be buffered before playback starts
 * (forwarded to the Rust scheduler). Larger than its 400ms default as an
 * experiment against the observed startup-transient underruns
 * (small dropouts clustered in the first ~1.2s of a stream): with the host's
 * send-ahead horizon now genuinely delivering content this far in advance,
 * a bigger cushion here gives the render ring and coroutine dispatcher more
 * real wall-clock time to reach steady cadence before playback begins,
 * rather than starting the instant the minimum threshold is technically met.
 */
private const val STARTUP_BUFFER_MS = 1_000L

/**
 * Platform hook keeping the device's network radio responsive for the
 * lifetime of one live connection. Without it, Android Wi-Fi power save may
 * buffer inbound packets at the access point for hundreds of milliseconds to
 * seconds at a time -- observed on a real device as multi-second arrival
 * outages mid-stream and as connection-start sync samples with RTTs far
 * above the estimator's acceptance bound. Implementations must be idempotent
 * in both directions.
 */
interface NetworkSessionLock {
    fun acquire()
    fun release()
}

/**
 * Android-facing wrapper around the shared Rust listener transport for one
 * manual-endpoint connection attempt at a time. Owns the transport, the real
 * playback of whatever it streams (via the Rust playback runtime), and
 * clock-sync probing for that same connection -- it makes no join/approval
 * domain decisions of its own, only consumes what the transport already
 * decided and projects it into [ManualConnectUiState].
 */
class ManualListenerTransportController(
    /**
     * Directory for debug PCM capture output (app-specific external storage,
     * pullable via `adb pull` without root). Null disables recording. This is
     * diagnostic-only instrumentation for this viability PoC, not a
     * production feature.
     */
    private val debugRecordingDirectory: File? = null,
    /** Held for the connection's lifetime; null when the platform provides none. */
    private val networkSessionLock: NetworkSessionLock? = null,
) : AutoCloseable {
    private val handleRef = AtomicReference<FfiListenerTransportHandle?>(null)
    private var eventLoop: Job? = null
    private var syncProbeJob: Job? = null
    private var diagnosticsSampleJob: Job? = null
    private var playbackRuntime: FfiListenerPlaybackHandle? = null
    private var currentStreamId: StreamId? = null

    private val _connectState = MutableStateFlow<ManualConnectUiState>(ManualConnectUiState.Idle)
    val connectState: StateFlow<ManualConnectUiState> = _connectState.asStateFlow()

    private var trustedForFuture: Boolean = false
    private var sessionId: SessionId? = null
    private var protocolVersion: Int = 0

    private val logger = AppLogger("ManualListenerAudio")
    private var lastReceivedSequence: Long? = null
    private var receivedCount: Long = 0
    private var writtenCount: Long = 0
    private var lastWrittenFrameConcealed = false

    suspend fun parse(rawInput: String): ManualEndpointParseResult = withContext(Dispatchers.Default) {
        try {
            ManualEndpointParseResult.Valid(parseManualHostEndpoint(rawInput, nowMs()))
        } catch (error: FfiListenerTransportException) {
            ManualEndpointParseResult.Invalid(error.message ?: "connection payload is invalid")
        }
    }

    suspend fun connect(
        scope: CoroutineScope,
        rawInput: String,
        localDeviceId: String,
        displayName: String,
        inviteCode: String?,
    ) {
        withContext(Dispatchers.IO) {
            // Stop any stream still running before tearing down its
            // transport: a failed reconnect otherwise leaves the previous
            // stream audibly playing behind a "connection failed" message.
            stopPlayback()
            closeExistingHandle()
            // Before the socket connect, not after approval: power-save
            // latency also poisons the first clock-sync exchanges, and those
            // begin the moment the connection is up.
            try {
                networkSessionLock?.acquire()
            } catch (error: RuntimeException) {
                logger.w("manual.network_lock", "acquire failed: ${error.message}")
            }
            _connectState.value = ManualConnectUiState.Connecting
            val endpoint = try {
                parseManualHostEndpoint(rawInput, nowMs())
            } catch (error: FfiListenerTransportException) {
                _connectState.value = ManualConnectUiState.Failed(error.message ?: "connection payload is invalid")
                return@withContext
            }
            sessionId = SessionId(endpoint.sessionId)
            protocolVersion = endpoint.protocolVersion.toInt()
            val handle = try {
                FfiListenerTransportHandle.connect(rawInput, nowMs(), localDeviceId, LOCAL_BIND_ADDRESS)
            } catch (error: FfiListenerTransportException) {
                _connectState.value = ManualConnectUiState.Failed(error.message ?: "connection failed")
                return@withContext
            }
            handleRef.set(handle)
            try {
                handle.sendJoinRequest(displayName, inviteCode)
            } catch (error: FfiListenerTransportException) {
                _connectState.value = ManualConnectUiState.Failed(error.message ?: "join request failed")
                closeExistingHandle()
                return@withContext
            }
            startEventLoop(scope, handle)
        }
    }

    suspend fun disconnect(reason: String) {
        withContext(Dispatchers.IO) {
            val handle = handleRef.get() ?: return@withContext
            runCatching { handle.sendDisconnect(reason) }
        }
    }

    fun reset() {
        eventLoop?.cancel()
        eventLoop = null
        stopPlayback()
        closeExistingHandle()
        _connectState.value = ManualConnectUiState.Idle
    }

    override fun close() {
        eventLoop?.cancel()
        eventLoop = null
        stopPlayback()
        closeExistingHandle()
    }

    private fun startEventLoop(scope: CoroutineScope, handle: FfiListenerTransportHandle) {
        eventLoop?.cancel()
        eventLoop = scope.launch(Dispatchers.IO) {
            while (isActive && handleRef.get() === handle) {
                val event = try {
                    handle.pollEvent(POLL_TIMEOUT_MS)
                } catch (error: FfiListenerTransportException) {
                    stopPlayback()
                    _connectState.value = mapPostConnectionFailure(error)
                    break
                }
                if (event != null) {
                    applyEvent(scope, handle, event)
                }
            }
        }
    }

    private fun applyEvent(
        scope: CoroutineScope,
        handle: FfiListenerTransportHandle,
        event: FfiListenerTransportEvent,
    ) {
        when (event) {
            is FfiListenerTransportEvent.Hello -> _connectState.value = ManualConnectUiState.AwaitingApproval(
                hostName = event.hostName,
                sessionName = event.sessionName,
            )
            is FfiListenerTransportEvent.JoinApproved -> {
                trustedForFuture = event.trustedForFuture
                _connectState.value = ManualConnectUiState.Approved(trustedForFuture)
                startSyncProbeLoop(scope, handle)
            }
            is FfiListenerTransportEvent.JoinRejected -> {
                stopPlayback()
                _connectState.value = ManualConnectUiState.Rejected(event.reason)
            }
            is FfiListenerTransportEvent.HostDisconnected -> {
                stopPlayback()
                _connectState.value = ManualConnectUiState.Disconnected(event.reason)
            }
            is FfiListenerTransportEvent.ConnectionClosed -> {
                stopPlayback()
                _connectState.value = ManualConnectUiState.Disconnected(event.message)
            }
            is FfiListenerTransportEvent.Rejected -> {
                stopPlayback()
                _connectState.value = ManualConnectUiState.Failed(event.message)
            }
            is FfiListenerTransportEvent.StreamStarted -> handleStreamStarted(scope, handle, event)
            is FfiListenerTransportEvent.Paused ->
                _connectState.value = ManualConnectUiState.Streaming(trustedForFuture, PlaybackState.PAUSED)
            is FfiListenerTransportEvent.Stopped -> handleStreamStopped()
            is FfiListenerTransportEvent.SyncResponseReceived -> handleSyncResponse(event)
            is FfiListenerTransportEvent.AudioReceived -> handleAudioReceived(event)
        }
    }

    /**
     * Samples playback diagnostics once a second for the life of one stream.
     *
     * The end-of-stream summary reports only totals, which cannot distinguish
     * a defect concentrated in the first second of playback from the same
     * count spread evenly across the whole stream -- and the two have
     * completely different causes. The debug WAV cannot settle it either: it
     * captures frames on their way *into* the render ring, so silence the
     * real-time callback substitutes on an empty ring is inaudible to it.
     * Per-second deltas are the only view that locates a hiccup in time.
     */
    private fun startDiagnosticsSampler(scope: CoroutineScope, runtime: FfiListenerPlaybackHandle) {
        diagnosticsSampleJob?.cancel()
        diagnosticsSampleJob = scope.launch(Dispatchers.IO) {
            var previousUnderruns = 0uL
            var previousSilenceFrames = 0uL
            var previousConcealed = 0uL
            var previousEmitted = 0uL
            while (isActive) {
                delay(DIAGNOSTICS_SAMPLE_CADENCE_MS)
                val diagnostics = playbackRuntime?.takeIf { it === runtime }?.diagnostics() ?: break
                appendDiagnosticsLine(
                    "sample emitted=+${diagnostics.packetsEmitted - previousEmitted} " +
                        "concealed=+${diagnostics.concealedPackets - previousConcealed} " +
                        "underruns=+${diagnostics.ringUnderruns - previousUnderruns} " +
                        "silenceFrames=+${diagnostics.ringSilenceFilledFrames - previousSilenceFrames} " +
                        "ringQueued=${diagnostics.ringQueuedFrames} " +
                        "bufferedMs=${diagnostics.bufferedSpanMs} phase=${diagnostics.phase}",
                )
                logger.i(
                    "manual.audio.sample",
                    "emitted=+${diagnostics.packetsEmitted - previousEmitted} " +
                        "concealed=+${diagnostics.concealedPackets - previousConcealed} " +
                        "underruns=+${diagnostics.ringUnderruns - previousUnderruns} " +
                        "silenceFrames=+${diagnostics.ringSilenceFilledFrames - previousSilenceFrames} " +
                        "ringQueued=${diagnostics.ringQueuedFrames} " +
                        "bufferedMs=${diagnostics.bufferedSpanMs} phase=${diagnostics.phase}",
                )
                previousUnderruns = diagnostics.ringUnderruns
                previousSilenceFrames = diagnostics.ringSilenceFilledFrames
                previousConcealed = diagnostics.concealedPackets
                previousEmitted = diagnostics.packetsEmitted
            }
        }
    }

    /**
     * Drives clock-sync probes for the life of one stream.
     *
     * The first probe goes out immediately rather than after a cadence delay:
     * playback cannot start until a sample is genuinely accepted, so the
     * round trip is on the critical path to hearing anything.
     */
    private fun startSyncProbeLoop(scope: CoroutineScope, handle: FfiListenerTransportHandle) {
        syncProbeJob?.cancel()
        syncProbeJob = scope.launch(Dispatchers.IO) {
            var correlationId = 1L
            while (isActive) {
                val runtime = playbackRuntime ?: break
                val sendTimeMs = runtime.nowMs()
                // Register before sending: a response that beats its own
                // registration has nothing to correlate against.
                val registered = runCatching {
                    runtime.beginSyncProbe(correlationId.toULong(), sendTimeMs)
                }.isSuccess
                if (registered) {
                    runCatching { handle.sendSyncRequest(correlationId.toULong(), sendTimeMs) }
                }
                correlationId += 1
                val locked = playbackRuntime?.diagnostics()?.syncLocked == true
                delay(if (locked) SYNC_PROBE_CADENCE_MS else SYNC_PROBE_ACQUIRE_CADENCE_MS)
            }
        }
    }

    /**
     * Forwards one four-timestamp exchange to the Rust estimator.
     *
     * `t4` is the moment this event is observed, taken from the runtime's own
     * clock so the estimate and the playback timeline share one timeline.
     * Nothing here interprets the sample: acceptance, offset, and skew are the
     * estimator's to decide, and a listener that computed any of them would be
     * duplicating domain logic it cannot keep consistent.
     */
    private fun handleSyncResponse(event: FfiListenerTransportEvent.SyncResponseReceived) {
        val runtime = playbackRuntime ?: return
        val outcome = runCatching {
            runtime.observeSyncResponse(
                event.correlationId,
                event.t1ListenerSendElapsedMs,
                event.t2HostReceiveElapsedMs,
                event.t3HostSendElapsedMs,
                runtime.nowMs(),
            )
        }.getOrElse { error ->
            logger.w("manual.audio.sync_rejected", error.message ?: "sync response rejected")
            return
        }
        // Teed to the durable file as well as logcat: distinguishing an
        // offset-driven rebuffer from a concealment-driven one needs the
        // per-sample offset series, and the offset path increments no
        // counter of its own (its result is discarded in
        // `observe_sync_response`), so this series is the only way to tell
        // the two causes apart on a device whose logcat is unavailable.
        appendDiagnosticsLine(
            "sync accepted=${outcome.accepted} offsetMs=${outcome.offsetMs} " +
                "rttMs=${outcome.roundTripTimeMs} jitterMs=${outcome.jitterMs} " +
                "samples=${outcome.acceptedSampleCount} locked=${outcome.syncLocked} " +
                "confidence=${outcome.confidence}",
        )
        logger.i(
            "manual.audio.sync_sample",
            "accepted=${outcome.accepted} offsetMs=${outcome.offsetMs} " +
                "skewPpm=${outcome.skewPpm} rttMs=${outcome.roundTripTimeMs} " +
                "samples=${outcome.acceptedSampleCount} syncLocked=${outcome.syncLocked}",
        )
        if (outcome.syncLocked && _connectState.value is ManualConnectUiState.Streaming) {
            _connectState.value = ManualConnectUiState.Streaming(trustedForFuture, PlaybackState.PLAYING)
        }
    }

    /**
     * Starts Rust-owned playback for one stream.
     *
     * Unlike the previous implementation this does not defer on sync: the
     * runtime refuses to play against a placeholder offset itself, so the
     * stream can be opened as soon as it is announced and will begin the
     * moment a sample is accepted.
     */
    private fun handleStreamStarted(scope: CoroutineScope, handle: FfiListenerTransportHandle, event: FfiListenerTransportEvent.StreamStarted) {
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
        endStream(keepOutputOpen = true)
        receivedCount = 0
        lastReceivedSequence = null

        val sampleRate = event.sampleRate.toInt()
        val samplesPerPacket = event.samplesPerPacket.toInt()
        val packetDurationMs = if (sampleRate > 0) {
            (samplesPerPacket.toLong() * 1_000L / sampleRate.toLong()).toInt()
        } else {
            0
        }
        val runtime = runCatching {
            FfiListenerPlaybackHandle.open(
                FfiListenerPlaybackConfig(
                    sessionId = session.value,
                    streamId = event.streamId,
                    packetDurationMs = packetDurationMs.toUInt(),
                    hostStartTimeMs = event.hostStartTimeMs,
                    samplesPerPacket = event.samplesPerPacket,
                    channels = event.channels,
                    startupBufferTargetMs = STARTUP_BUFFER_MS.toULong(),
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
        startDebugCapture(runtime, event.streamId)
        startDiagnosticsSampler(scope, runtime)
        logger.i(
            "manual.audio.stream_started",
            "streamId=${event.streamId} hostStartMs=${event.hostStartTimeMs} " +
                "sampleRate=$sampleRate samplesPerPacket=$samplesPerPacket packetDurationMs=$packetDurationMs",
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
    /**
     * Appends one diagnostic line to a durable file beside the debug PCM
     * capture, when a recording directory is configured.
     *
     * `AppLogger` reaches only logcat, which is entirely unavailable on some
     * real devices (confirmed on an Android 8.0 phone: every buffer empty,
     * `ro.logdumpd.enabled=0`). Physical-device validation is exactly where
     * these numbers matter most, and the in-app diagnostics screen cannot be
     * reached mid-stream without tearing the session down -- so without a
     * file sink a real run's listener-side counters are simply unobservable.
     */
    private fun appendDiagnosticsLine(line: String) {
        val directory = debugRecordingDirectory ?: return
        runCatching {
            File(directory, DIAGNOSTICS_LOG_FILE_NAME)
                .appendText("[t=${SystemClock.elapsedRealtime()}] $line\n")
        }
    }

    private fun startDebugCapture(runtime: FfiListenerPlaybackHandle, streamId: String) {
        val directory = debugRecordingDirectory ?: return
        val file = File(directory, "manual-listener-$streamId.wav")
        runCatching { runtime.startDebugCapture(file.absolutePath) }
            .onSuccess { logger.i("manual.audio.recording_started", file.absolutePath) }
            .onFailure { error ->
                logger.w("manual.audio.recording_failed", error.message ?: "debug capture failed to start")
            }
    }

    private fun handleStreamStopped() {
        // The runtime drains its own buffered tail as part of stopping, so a
        // stream's final moments are played rather than discarded. The native
        // output deliberately stays open: the session is still live and the
        // host may start another track, which then rebinds this same stream
        // instead of reopening one (see [endStream]).
        endStream(keepOutputOpen = true)
        _connectState.value = ManualConnectUiState.Approved(trustedForFuture)
    }

    /**
     * Fully tears down playback *and* the native output. For ending the
     * current stream while the session continues, use
     * `endStream(keepOutputOpen = true)` instead.
     */
    private fun stopPlayback() = endStream(keepOutputOpen = false)

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
    private fun endStream(keepOutputOpen: Boolean) {
        syncProbeJob?.cancel()
        syncProbeJob = null
        diagnosticsSampleJob?.cancel()
        diagnosticsSampleJob = null
        val runtime = playbackRuntime ?: run {
            // No stream to end, but a teardown still has to release the
            // output; leaving a low-latency stream running against a
            // released token would burn the radio and the CPU for silence.
            if (!keepOutputOpen) OboeBridge.nativeOboeClose()
            return
        }
        playbackRuntime = null
        currentStreamId = null
        // Order matters and is deliberate: `stop()` drains the render ring
        // *through the still-running Oboe callback* so the stream ends on its
        // own final sample rather than mid-note (see `await_ring_drain`, which
        // documents that a consumer closed first means the ring never drains).
        // The window between the token's release inside `stop()` and either
        // the close below or the next track's rebind is contained by the ABI
        // itself -- a released token reads as silence, never as freed memory.
        runCatching { runtime.stop() }.onFailure { error ->
            logger.w("manual.audio.stop_failed", error.message ?: "playback failed to stop cleanly")
        }
        if (!keepOutputOpen) OboeBridge.nativeOboeClose()
        logPlaybackSummary(runtime)
        runtime.close()
    }

    /**
     * Logs the stream's final accounting from the Rust runtime.
     *
     * Every counter here is produced by the component that owns the
     * behaviour it describes, so the summary cannot disagree with what
     * actually happened the way an independently maintained tally could.
     */
    private fun logPlaybackSummary(runtime: FfiListenerPlaybackHandle) {
        val diagnostics = runtime.finalDiagnostics() ?: runtime.diagnostics()
        appendDiagnosticsLine(
            "summary streamId=$currentStreamId received=$receivedCount " +
                "accepted=${diagnostics.packetsAccepted} emitted=${diagnostics.packetsEmitted} " +
                "concealed=${diagnostics.concealedPackets} late=${diagnostics.lateRejections} " +
                "skipped=${diagnostics.sequencesSkipped} " +
                "droppedBeforeSync=${diagnostics.droppedBeforeSync} " +
                "hardResyncs=${diagnostics.hardResyncSignals} " +
                "ringUnderruns=${diagnostics.ringUnderruns} " +
                "ringSilenceFilled=${diagnostics.ringSilenceFilledFrames} " +
                "ringFullEvents=${diagnostics.ringFullEvents} " +
                "ringPeakFrames=${diagnostics.ringPeakQueuedFrames} " +
                "prefillFrames=${diagnostics.prefillFrames} phase=${diagnostics.phase} " +
                "oboe=${OboeBridge.lastOpenSummary()}",
        )
        logger.i(
            "manual.audio.summary",
            "received=$receivedCount accepted=${diagnostics.packetsAccepted} " +
                "emitted=${diagnostics.packetsEmitted} skipped=${diagnostics.sequencesSkipped} " +
                "concealed=${diagnostics.concealedPackets} late=${diagnostics.lateRejections} " +
                "duplicate=${diagnostics.duplicateRejections} " +
                "reorderWindow=${diagnostics.reorderWindowRejections} " +
                "hardResyncs=${diagnostics.hardResyncSignals} " +
                "resyncs=${diagnostics.resynchronisations} " +
                "droppedBeforeSync=${diagnostics.droppedBeforeSync} " +
                "ringPeakFrames=${diagnostics.ringPeakQueuedFrames} " +
                "prefillFrames=${diagnostics.prefillFrames} " +
                "ringUnderruns=${diagnostics.ringUnderruns} " +
                "ringSilenceFilled=${diagnostics.ringSilenceFilledFrames} " +
                "ringFullEvents=${diagnostics.ringFullEvents} phase=${diagnostics.phase}",
        )
        runtime.debugCaptureError()?.let { error ->
            logger.w("manual.audio.recording_error", error)
        }
    }

    private fun handleAudioReceived(event: FfiListenerTransportEvent.AudioReceived) {
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
        }
    }

    private fun handlePlaybackEngineFailure(error: Throwable) {
        stopPlayback()
        _connectState.value = ManualConnectUiState.Failed(error.message ?: "playback engine failed")
    }

    /** Stops this connection's real-time consumption -- playback, buffering, and sync probing. */
    private fun closeExistingHandle() {
        handleRef.getAndSet(null)?.let { handle ->
            runCatching { handle.shutdown() }
            handle.close()
        }
        try {
            networkSessionLock?.release()
        } catch (error: RuntimeException) {
            logger.w("manual.network_lock", "release failed: ${error.message}")
        }
    }

    private fun nowMs(): ULong = System.currentTimeMillis().toULong()
}

/**
 * Maps any exception surfaced while polling an already-connected transport.
 *
 * The poll loop only starts after `connect()` and `sendJoinRequest()` have already
 * succeeded, so every exception it can observe - including `Closed`/`ShuttingDown` -
 * represents the connection ending, never a fresh configuration failure. Mapping any
 * of them to `Failed` would render a real "host ended session" as an indistinguishable
 * connection error.
 */
internal fun mapPostConnectionFailure(
    error: FfiListenerTransportException,
): ManualConnectUiState.Disconnected = ManualConnectUiState.Disconnected(error.message)
