package com.ekkus.silentdisco.core.rust

import android.os.SystemClock
import com.ekkus.silentdisco.core.audio.AudioFormatSpec
import com.ekkus.silentdisco.core.audio.DebugPcmRecorder
import com.ekkus.silentdisco.core.audio.ListenerPlaybackScheduler
import com.ekkus.silentdisco.core.audio.OboeBridge
import com.ekkus.silentdisco.core.audio.OboePlaybackEngine
import com.ekkus.silentdisco.core.audio.PlaybackEngine
import com.ekkus.silentdisco.core.audio.PlaybackFrame
import com.ekkus.silentdisco.core.audio.PlaybackTelemetry
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
private const val MAX_PENDING_PACKETS = 256
private const val PLAYBACK_RETRY_DELAY_MS = 10L

/**
 * How far ahead of each frame's presentation deadline it is written into the
 * render ring. The ring's steady-state depth converges to this lead, giving
 * the native Oboe callback a real cushion instead of the near-zero depth
 * that write-at-deadline pacing left it (any writer-side jitter -- coroutine
 * scheduling, the ~5ms JNI boxing per write, GC -- then starved the DAC for
 * ~2ms at a time: hard native cuts no Kotlin-side concealment ramp can
 * reach). Matches the ring's 400ms target fill; capacity is 1s.
 */
private const val RING_WRITE_LEAD_MS = 400L

/**
 * Upper bound on the stream-start silence prefill that aligns the first
 * frame's ring position with its presentation deadline. Below ring capacity
 * (1s) so the prefill plus the initial lookahead pop burst can never
 * overflow the ring: max depth is max(prefill, lead) <= 800ms < 1s.
 */
private const val MAX_RING_PREFILL_MS = 800L

/**
 * Ring depth (48kHz frames, 400ms) at which the playback loop stops writing
 * and waits for the native consumer to drain. Without this cap, a large
 * startup backlog flushed through the lookahead pins the ring at full
 * capacity for the whole stream -- maximal latency and a write stall on
 * every single packet -- because full-ring backpressure is the only thing
 * left to pace the writer. Matches the ring's configured target fill.
 */
private const val RING_TARGET_DEPTH_FRAMES = 19_200

/**
 * Milliseconds of silence to queue before the first real frame so it plays
 * exactly at its deadline: the native callback consumes from the moment the
 * stream opens, so a frame's play time is its write time plus current ring
 * depth. Zero when the first frame is already due or late (the startup
 * backlog case), preserving the existing play-immediately behavior.
 */
internal fun computeRingPrefillMs(
    firstDeadlineMs: Long?,
    nowMs: Long,
    maxPrefillMs: Long = MAX_RING_PREFILL_MS,
): Long = ((firstDeadlineMs ?: nowMs) - nowMs).coerceIn(0, maxPrefillMs)
/**
 * How much scheduled-timeline span must be buffered before playback starts
 * (see [PlaybackThresholds.startupBufferMs]). Larger than the class default
 * (400ms) as an experiment against the observed startup-transient underruns
 * (small dropouts clustered in the first ~1.2s of a stream): with the host's
 * send-ahead horizon now genuinely delivering content this far in advance,
 * a bigger cushion here gives the render ring and coroutine dispatcher more
 * real wall-clock time to reach steady cadence before playback begins,
 * rather than starting the instant the minimum threshold is technically met.
 */
private const val STARTUP_BUFFER_MS = 1_000L

/** A stream announced before any real clock-sync sample landed; see [ManualListenerTransportController.beginPlayback]. */
private data class PendingStream(val streamId: StreamId, val sampleRate: Int, val channelCount: Int)

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
 * playback of whatever it streams (via the shared [playbackEngine]), and
 * clock-sync probing for that same connection -- it makes no join/approval
 * domain decisions of its own, only consumes what the transport already
 * decided and projects it into [ManualConnectUiState].
 */
class ManualListenerTransportController(
    private val playbackEngine: PlaybackEngine,
    /**
     * Directory for [DebugPcmRecorder] output (app-specific external storage,
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
    private var playbackJob: Job? = null
    private var debugRecorder: DebugPcmRecorder? = null

    private val _connectState = MutableStateFlow<ManualConnectUiState>(ManualConnectUiState.Idle)
    val connectState: StateFlow<ManualConnectUiState> = _connectState.asStateFlow()

    private var trustedForFuture: Boolean = false
    private var sessionId: SessionId? = null
    private var protocolVersion: Int = 0
    private var syncController: ListenerSyncController? = null
    private var currentSyncState: SyncState = SyncState()
    private var hasSyncSample: Boolean = false
    private var pendingStream: PendingStream? = null
    private var listenerScheduler: ListenerPlaybackScheduler? = null
    private val pendingPackets = ArrayDeque<AudioPacket>()

    private val logger = AppLogger("ManualListenerAudio")
    private var lastReceivedSequence: Long? = null
    private var receivedCount: Long = 0
    private var writtenCount: Long = 0
    private var lastWrittenFrameConcealed = false
    private var lastTelemetry: PlaybackTelemetry = PlaybackTelemetry()

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
        stopPlaybackAndSync()
        closeExistingHandle()
        _connectState.value = ManualConnectUiState.Idle
    }

    override fun close() {
        eventLoop?.cancel()
        eventLoop = null
        stopPlaybackAndSync()
        closeExistingHandle()
    }

    private fun startEventLoop(scope: CoroutineScope, handle: FfiListenerTransportHandle) {
        eventLoop?.cancel()
        eventLoop = scope.launch(Dispatchers.IO) {
            while (isActive && handleRef.get() === handle) {
                val event = try {
                    handle.pollEvent(POLL_TIMEOUT_MS)
                } catch (error: FfiListenerTransportException) {
                    stopPlaybackAndSync()
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
                stopPlaybackAndSync()
                _connectState.value = ManualConnectUiState.Rejected(event.reason)
            }
            is FfiListenerTransportEvent.HostDisconnected -> {
                stopPlaybackAndSync()
                _connectState.value = ManualConnectUiState.Disconnected(event.reason)
            }
            is FfiListenerTransportEvent.ConnectionClosed -> {
                stopPlaybackAndSync()
                _connectState.value = ManualConnectUiState.Disconnected(event.message)
            }
            is FfiListenerTransportEvent.Rejected -> {
                stopPlaybackAndSync()
                _connectState.value = ManualConnectUiState.Failed(event.message)
            }
            is FfiListenerTransportEvent.StreamStarted -> handleStreamStarted(scope, event)
            is FfiListenerTransportEvent.Paused ->
                _connectState.value = ManualConnectUiState.Streaming(trustedForFuture, PlaybackState.PAUSED)
            is FfiListenerTransportEvent.Stopped -> handleStreamStopped()
            is FfiListenerTransportEvent.SyncResponseReceived -> handleSyncResponse(scope, event)
            is FfiListenerTransportEvent.AudioReceived -> handleAudioReceived(event)
        }
    }

    private fun startSyncProbeLoop(scope: CoroutineScope, handle: FfiListenerTransportHandle) {
        val session = sessionId ?: return
        val config = SyncMaintenanceConfig()
        syncController = ListenerSyncController(sessionId = session, config = config)
        syncProbeJob?.cancel()
        syncProbeJob = scope.launch(Dispatchers.IO) {
            while (isActive) {
                val controller = syncController ?: break
                val probe = controller.newProbe()
                runCatching {
                    handle.sendSyncRequest(probe.correlationId.toULong(), probe.t1ListenerSendElapsedMs.toULong())
                }
                delay(config.cadenceMs)
            }
        }
    }

    private fun handleSyncResponse(scope: CoroutineScope, event: FfiListenerTransportEvent.SyncResponseReceived) {
        val session = sessionId ?: return
        val controller = syncController ?: return
        val t1 = event.t1ListenerSendElapsedMs.toLong()
        val t2 = event.t2HostReceiveElapsedMs.toLong()
        val t3 = event.t3HostSendElapsedMs.toLong()
        val t4 = SystemClock.elapsedRealtime()
        currentSyncState = controller.onResponse(
            SyncResponsePacket(
                version = protocolVersion,
                sessionId = session,
                correlationId = event.correlationId.toLong(),
                t1ListenerSendElapsedMs = t1,
                t2HostReceiveElapsedMs = t2,
                t3HostSendElapsedMs = t3,
            ),
        )
        logger.i(
            "manual.audio.sync_sample",
            "t1=$t1 t2=$t2 t3=$t3 t4approx=$t4 rttMs=${(t4 - t1) - (t3 - t2)} " +
                "offsetMs=${currentSyncState.offsetMs} skewPpm=${currentSyncState.skewPpm} " +
                "confidence=${currentSyncState.confidence} hasSyncSample=$hasSyncSample",
        )
        // A response event arriving is not the same as the estimator having
        // accepted a usable sample from it: ClockSyncEstimator silently
        // rejects RTT outliers and falls back to its all-zero default
        // SyncState in that case. confidence stays UNKNOWN exactly when no
        // sample has ever been accepted, so gate on that instead of merely
        // "an event arrived" -- otherwise a rejected first sample (e.g. an
        // elevated RTT from connection-start contention) freezes playback's
        // mapper on the garbage zero-offset default this whole deferred-start
        // scheme exists to avoid.
        if (!hasSyncSample && currentSyncState.confidence != SyncQualityBadge.UNKNOWN) {
            hasSyncSample = true
            pendingStream?.let { pending ->
                pendingStream = null
                beginPlayback(scope, pending.streamId, pending.sampleRate, pending.channelCount)
            }
        }
    }

    /**
     * A stream can be announced before this connection has ever completed a
     * real clock-sync round trip (e.g. a host that starts playback moments
     * after approving a listener). The default sync estimate (0ms offset) is
     * essentially guaranteed wrong -- the host's and this device's monotonic
     * clocks have unrelated epochs (process start vs. device boot) -- so
     * starting playback against it would schedule every packet nonsensically
     * (either all immediately "late" or all far in the future) with no
     * audible result. Defer until [handleSyncResponse] reports the first
     * real sample instead of ever building a mapper from a guess.
     */
    private fun handleStreamStarted(scope: CoroutineScope, event: FfiListenerTransportEvent.StreamStarted) {
        val streamId = StreamId(event.streamId)
        val sampleRate = event.sampleRate.toInt()
        val channelCount = event.channels.toInt()
        if (!hasSyncSample) {
            pendingStream = PendingStream(streamId, sampleRate, channelCount)
            _connectState.value = ManualConnectUiState.Streaming(trustedForFuture, PlaybackState.BUFFERING)
            return
        }
        beginPlayback(scope, streamId, sampleRate, channelCount)
    }

    /**
     * Starts real playback of a stream once a real sync estimate is known.
     * The mapper is built once from whatever sync estimate exists right now
     * and frozen for this stream's whole lifetime -- later sync samples
     * refine `currentSyncState` for the *next* stream, matching this
     * codebase's existing accepted behavior for the BLE/Wi-Fi-Direct
     * discovered-session path.
     */
    private fun beginPlayback(scope: CoroutineScope, streamId: StreamId, sampleRate: Int, channelCount: Int) {
        val session = sessionId ?: return
        receivedCount = 0
        writtenCount = 0
        lastReceivedSequence = null
        lastTelemetry = PlaybackTelemetry()
        lastWrittenFrameConcealed = false
        val mapper = HostTimeMapper(offsetMs = currentSyncState.offsetMs, skewPpm = currentSyncState.skewPpm)
        logger.i(
            "manual.audio.stream_mapper",
            "streamId=${streamId.value} offsetMs=${currentSyncState.offsetMs} " +
                "skewPpm=${currentSyncState.skewPpm} pendingPacketsBuffered=${pendingPackets.size}",
        )
        val scheduler = ListenerPlaybackScheduler(
            mapper = mapper,
            thresholds = PlaybackThresholds(startupBufferMs = STARTUP_BUFFER_MS),
            expectedSessionId = session,
            expectedStreamId = streamId,
        )
        listenerScheduler = scheduler
        pendingPackets
            .filter { it.sessionId == session && it.streamId == streamId }
            .forEach { scheduler.submit(it) }
        pendingPackets.clear()
        val format = AudioFormatSpec(sampleRate = sampleRate, channelCount = channelCount)
        runCatching { playbackEngine.start(format) }.onFailure { error ->
            handlePlaybackEngineFailure(error)
            return
        }
        startDebugRecording(streamId, sampleRate, channelCount)
        _connectState.value = ManualConnectUiState.Streaming(trustedForFuture, PlaybackState.BUFFERING)
        playbackJob?.cancel()
        playbackJob = scope.launch(Dispatchers.IO) {
            var started = false
            while (isActive) {
                val activeScheduler = listenerScheduler ?: return@launch
                if (!started) {
                    if (!activeScheduler.canStart()) {
                        delay(PLAYBACK_RETRY_DELAY_MS)
                        continue
                    }
                    started = true
                    // Align the first frame's ring position with its deadline:
                    // the native callback has been consuming (silence-filling)
                    // since engine start, so whatever is written next plays
                    // almost immediately. Queuing exactly (deadline - now) of
                    // silence first makes the first frame play on time and
                    // seeds the ring's intentional depth.
                    val prefillNowMs = SystemClock.elapsedRealtime()
                    val prefillMs = computeRingPrefillMs(activeScheduler.nextDeadlineMs(), prefillNowMs)
                    if (prefillMs > 0) {
                        val prefillFrames = (prefillMs * format.sampleRate / 1_000L).toInt()
                        val prefilled = runCatching { playbackEngine.prefillSilence(prefillFrames) }
                            .getOrElse { error ->
                                handlePlaybackEngineFailure(error)
                                return@launch
                            }
                        logger.i(
                            "manual.audio.ring_prefill",
                            "prefillMs=$prefillMs requestedFrames=$prefillFrames prefilledFrames=$prefilled",
                        )
                    }
                    _connectState.value = ManualConnectUiState.Streaming(trustedForFuture, PlaybackState.PLAYING)
                }
                // Never write past the ring's intended depth: with the cap,
                // even a large startup backlog settles the ring at the target
                // cushion instead of pinning it full (maximal latency, a
                // write stall on every packet).
                if (playbackEngine.queuedDepthFrames() >= RING_TARGET_DEPTH_FRAMES) {
                    delay(PLAYBACK_RETRY_DELAY_MS)
                    continue
                }
                // Frames are released RING_WRITE_LEAD_MS ahead of their
                // presentation deadline: the ring's FIFO position -- not the
                // write moment -- decides when a frame actually plays, so the
                // early hand-off keeps steady-state ring depth at the lead
                // (a jitter cushion for the native consumer) without moving
                // any frame's audible timing. Draining everything inside the
                // lead window immediately, and only waiting when nothing is,
                // avoids compounding per-loop overhead into drift.
                val frame = activeScheduler.poll(SystemClock.elapsedRealtime() + RING_WRITE_LEAD_MS)
                if (frame == null) {
                    delay(PLAYBACK_RETRY_DELAY_MS)
                    continue
                }
                writeFrame(frame)?.let { error ->
                    handlePlaybackEngineFailure(error)
                    return@launch
                }
            }
        }
    }

    /**
     * Writes one already-scheduled frame to the engine and the debug
     * recorder, incrementing [writtenCount] on success. Returns the write's
     * exception on failure (null on success) instead of handling it, so the
     * main playback loop can treat it as a fatal engine failure while a
     * deliberate stop-time drain (see [handleStreamStopped]) can just stop
     * draining rather than running the full failure path for an engine
     * that's already being torn down.
     */
    private fun writeFrame(frame: PlaybackFrame): Throwable? {
        // One log line per concealment *run*, not per concealed frame: a
        // multi-second arrival outage otherwise emits ~50 warning lines per
        // second from the playback thread, and that logging load lands on the
        // same process already struggling to keep audio flowing.
        if (frame.concealed && !lastWrittenFrameConcealed) {
            logger.w("manual.audio.concealed_frame", "concealment run started at seq=${frame.packet.sequenceNumber}")
        }
        lastWrittenFrameConcealed = frame.concealed
        // Recorded exactly as handed to the engine -- same payload, same
        // order -- so the saved file reflects precisely what this app
        // believed it was playing, concealment included.
        debugRecorder?.append(frame.packet.payload)
        val result = runCatching { playbackEngine.write(frame) }
        result.onSuccess { writtenCount += 1 }
        return result.exceptionOrNull()
    }

    private fun startDebugRecording(streamId: StreamId, sampleRate: Int, channelCount: Int) {
        val directory = debugRecordingDirectory ?: return
        debugRecorder?.finish()
        val file = File(directory, "manual-listener-${streamId.value}.wav")
        val recorder = DebugPcmRecorder(file)
        recorder.start(sampleRate, channelCount)
        debugRecorder = recorder
        logger.i("manual.audio.recording_started", file.absolutePath)
    }

    private fun finishDebugRecording() {
        val recorder = debugRecorder ?: return
        debugRecorder = null
        recorder.finish()
    }

    private fun handleStreamStopped() {
        playbackJob?.cancel()
        playbackJob = null
        // Anything still buffered here already arrived over the network in
        // time -- it's real tail content (e.g. a song's final note), not
        // backlog. The send-ahead horizon means up to roughly a second of
        // it can legitimately be sitting here at any moment, so play it out
        // before tearing down instead of discarding it silently.
        listenerScheduler?.drainRemaining()?.forEach { frame -> writeFrame(frame) }
        logPlaybackSummary()
        finishDebugRecording()
        listenerScheduler = null
        pendingPackets.clear()
        runCatching { playbackEngine.stop() }
        _connectState.value = ManualConnectUiState.Approved(trustedForFuture)
    }

    /**
     * Logs everything needed to objectively tell where audio went missing or
     * wrong, rather than relying on a description of what it sounded like:
     * how many packets actually arrived vs. were written to the engine, the
     * scheduler's own loss/drop/conceal counters, and the native Oboe ring's
     * own underrun/silence-fill counters (0 unless the ring genuinely ran
     * dry -- confirms or rules out real-time starvation independently of
     * anything the Kotlin-side scheduler observed).
     */
    private fun logPlaybackSummary() {
        val stallSummary = (playbackEngine as? OboePlaybackEngine)?.stallSummary()
        logger.i(
            "manual.audio.summary",
            "received=$receivedCount written=$writtenCount " +
                "packetLoss=${lastTelemetry.packetLossCount} lateDrop=${lastTelemetry.lateDropCount} " +
                "invalid=${lastTelemetry.invalidPacketCount} concealed=${lastTelemetry.concealedPacketCount} " +
                "oboeUnderruns=${OboeBridge.nativeOboeUnderrunCount()} " +
                "oboeSilenceFilledFrames=${OboeBridge.nativeOboeSilenceFilledFrames()} " +
                "oboeFramesRendered=${OboeBridge.nativeOboeFramesRendered()}" +
                (stallSummary?.let { " $it" } ?: ""),
        )
    }

    private fun handleAudioReceived(event: FfiListenerTransportEvent.AudioReceived) {
        val session = sessionId ?: return
        val packet = mapAudioReceivedToPacket(event, session, protocolVersion)
        receivedCount += 1
        val previousSequence = lastReceivedSequence
        if (previousSequence != null && packet.sequenceNumber != previousSequence + 1) {
            logger.w(
                "manual.audio.received_gap",
                "expected seq ${previousSequence + 1} but received ${packet.sequenceNumber} " +
                    "(network loss or reorder before the scheduler ever sees it)",
            )
        }
        lastReceivedSequence = packet.sequenceNumber
        val scheduler = listenerScheduler
        if (scheduler == null) {
            pendingPackets += packet
            while (pendingPackets.size > MAX_PENDING_PACKETS) {
                pendingPackets.removeFirst()
            }
            return
        }
        logTelemetryChange(scheduler.submit(packet), packet.sequenceNumber)
    }

    /**
     * [ListenerPlaybackScheduler.submit] returns a running telemetry
     * snapshot, not just this call's outcome -- logs only the counters that
     * actually changed since the last observation, tagged with the sequence
     * number that caused the change, so a real drop/late-drop/invalid/
     * concealment event is traceable to a specific packet instead of just a
     * final aggregate count.
     */
    private fun logTelemetryChange(telemetry: PlaybackTelemetry, sequenceNumber: Long) {
        if (telemetry.packetLossCount != lastTelemetry.packetLossCount) {
            logger.w("manual.audio.packet_loss", "seq=$sequenceNumber total=${telemetry.packetLossCount}")
        }
        if (telemetry.lateDropCount != lastTelemetry.lateDropCount) {
            logger.w("manual.audio.late_drop", "seq=$sequenceNumber total=${telemetry.lateDropCount}")
        }
        if (telemetry.invalidPacketCount != lastTelemetry.invalidPacketCount) {
            logger.w("manual.audio.invalid_packet", "seq=$sequenceNumber total=${telemetry.invalidPacketCount}")
        }
        if (telemetry.concealedPacketCount != lastTelemetry.concealedPacketCount) {
            logger.w("manual.audio.concealed", "seq=$sequenceNumber total=${telemetry.concealedPacketCount}")
        }
        lastTelemetry = telemetry
    }

    private fun handlePlaybackEngineFailure(error: Throwable) {
        stopPlaybackAndSync()
        _connectState.value = ManualConnectUiState.Failed(error.message ?: "playback engine failed")
    }

    /** Stops this connection's real-time consumption -- playback, buffering, and sync probing. */
    private fun stopPlaybackAndSync() {
        playbackJob?.cancel()
        playbackJob = null
        syncProbeJob?.cancel()
        syncProbeJob = null
        if (listenerScheduler != null) {
            logPlaybackSummary()
            finishDebugRecording()
            runCatching { playbackEngine.stop() }
        }
        listenerScheduler = null
        syncController = null
        hasSyncSample = false
        pendingStream = null
        pendingPackets.clear()
    }

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

/** Maps one raw inbound audio event into the shared [AudioPacket] model the scheduler consumes. */
internal fun mapAudioReceivedToPacket(
    event: FfiListenerTransportEvent.AudioReceived,
    sessionId: SessionId,
    protocolVersion: Int,
): AudioPacket = AudioPacket(
    version = protocolVersion,
    sessionId = sessionId,
    streamId = StreamId(event.streamId),
    sequenceNumber = event.sequence.toLong(),
    codec = AUDIO_CODEC_NAME,
    sampleRate = event.sampleRate.toInt(),
    channelCount = event.channels.toInt(),
    samplesPerPacket = event.samplesPerPacket.toInt(),
    firstSampleIndex = event.firstSampleIndex.toLong(),
    hostPresentationTimeMs = event.hostPresentationTimeMs.toLong(),
    payload = event.payload,
)

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
