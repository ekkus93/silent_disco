package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.audio.AudioFormatSpec
import com.ekkus.silentdisco.core.audio.ListenerPlaybackScheduler
import com.ekkus.silentdisco.core.audio.PlaybackEngine
import com.ekkus.silentdisco.core.audio.PlaybackThresholds
import com.ekkus.silentdisco.core.model.ManualConnectUiState
import com.ekkus.silentdisco.core.model.PlaybackState
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
private const val PLAYBACK_FRAME_INTERVAL_MS = 20L

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
) : AutoCloseable {
    private val handleRef = AtomicReference<FfiListenerTransportHandle?>(null)
    private var eventLoop: Job? = null
    private var syncProbeJob: Job? = null
    private var playbackJob: Job? = null

    private val _connectState = MutableStateFlow<ManualConnectUiState>(ManualConnectUiState.Idle)
    val connectState: StateFlow<ManualConnectUiState> = _connectState.asStateFlow()

    private var trustedForFuture: Boolean = false
    private var sessionId: SessionId? = null
    private var protocolVersion: Int = 0
    private var syncController: ListenerSyncController? = null
    private var currentSyncState: SyncState = SyncState()
    private var listenerScheduler: ListenerPlaybackScheduler? = null
    private val pendingPackets = ArrayDeque<AudioPacket>()

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
            is FfiListenerTransportEvent.SyncResponseReceived -> handleSyncResponse(event)
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

    private fun handleSyncResponse(event: FfiListenerTransportEvent.SyncResponseReceived) {
        val session = sessionId ?: return
        val controller = syncController ?: return
        currentSyncState = controller.onResponse(
            SyncResponsePacket(
                version = protocolVersion,
                sessionId = session,
                correlationId = event.correlationId.toLong(),
                t1ListenerSendElapsedMs = event.t1ListenerSendElapsedMs.toLong(),
                t2HostReceiveElapsedMs = event.t2HostReceiveElapsedMs.toLong(),
                t3HostSendElapsedMs = event.t3HostSendElapsedMs.toLong(),
            ),
        )
    }

    /**
     * Starts real playback of a just-announced stream. The mapper is built
     * once from whatever sync estimate exists right now and frozen for this
     * stream's whole lifetime -- later sync samples refine `currentSyncState`
     * for the *next* stream, matching this codebase's existing accepted
     * behavior for the BLE/Wi-Fi-Direct discovered-session path.
     */
    private fun handleStreamStarted(scope: CoroutineScope, event: FfiListenerTransportEvent.StreamStarted) {
        val session = sessionId ?: return
        val streamId = StreamId(event.streamId)
        val mapper = HostTimeMapper(offsetMs = currentSyncState.offsetMs, skewPpm = currentSyncState.skewPpm)
        val scheduler = ListenerPlaybackScheduler(
            mapper = mapper,
            thresholds = PlaybackThresholds(),
            expectedSessionId = session,
            expectedStreamId = streamId,
        )
        listenerScheduler = scheduler
        pendingPackets
            .filter { it.sessionId == session && it.streamId == streamId }
            .forEach { scheduler.submit(it) }
        pendingPackets.clear()
        val format = AudioFormatSpec(sampleRate = event.sampleRate.toInt(), channelCount = event.channels.toInt())
        runCatching { playbackEngine.start(format) }.onFailure { error ->
            handlePlaybackEngineFailure(error)
            return
        }
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
                    _connectState.value = ManualConnectUiState.Streaming(trustedForFuture, PlaybackState.PLAYING)
                }
                val frame = activeScheduler.poll()
                if (frame == null) {
                    delay(PLAYBACK_RETRY_DELAY_MS)
                    continue
                }
                runCatching { playbackEngine.write(frame) }.onFailure { error ->
                    handlePlaybackEngineFailure(error)
                    return@launch
                }
                delay(PLAYBACK_FRAME_INTERVAL_MS)
            }
        }
    }

    private fun handleStreamStopped() {
        playbackJob?.cancel()
        playbackJob = null
        listenerScheduler = null
        pendingPackets.clear()
        runCatching { playbackEngine.stop() }
        _connectState.value = ManualConnectUiState.Approved(trustedForFuture)
    }

    private fun handleAudioReceived(event: FfiListenerTransportEvent.AudioReceived) {
        val session = sessionId ?: return
        val packet = mapAudioReceivedToPacket(event, session, protocolVersion)
        val scheduler = listenerScheduler
        if (scheduler == null) {
            pendingPackets += packet
            while (pendingPackets.size > MAX_PENDING_PACKETS) {
                pendingPackets.removeFirst()
            }
            return
        }
        scheduler.submit(packet)
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
            runCatching { playbackEngine.stop() }
        }
        listenerScheduler = null
        syncController = null
        pendingPackets.clear()
    }

    private fun closeExistingHandle() {
        handleRef.getAndSet(null)?.let { handle ->
            runCatching { handle.shutdown() }
            handle.close()
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
