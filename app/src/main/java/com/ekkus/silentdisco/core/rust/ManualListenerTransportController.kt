package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.logging.AppLogger
import com.ekkus.silentdisco.core.model.ManualConnectUiState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
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
 *
 * Split across four files in this package, all operating on this same
 * class: this file (connection lifecycle: connect/disconnect/reset/close,
 * the event loop, and event dispatch), [startSyncProbeLoop] and friends in
 * `ManualListenerTransportControllerSync.kt` (clock-sync probing),
 * [handleStreamStarted] and friends in
 * `ManualListenerTransportControllerPlayback.kt` (stream/playback
 * lifecycle and inbound audio), and [startDiagnosticsSampler] and friends in
 * `ManualListenerTransportControllerDiagnostics.kt` (per-second sampling and
 * the end-of-stream summary). Fields those extension functions read or
 * mutate are `internal` rather than `private` for exactly that reason;
 * everything reachable only from this file stays `private`.
 */
class ManualListenerTransportController(
    /**
     * Directory for debug PCM capture output (app-specific external storage,
     * pullable via `adb pull` without root). Null disables recording. This is
     * diagnostic-only instrumentation for this viability PoC, not a
     * production feature.
     */
    internal val debugRecordingDirectory: File? = null,
    /** Held for the connection's lifetime; null when the platform provides none. */
    private val networkSessionLock: NetworkSessionLock? = null,
) : AutoCloseable {
    internal val handleRef = AtomicReference<FfiListenerTransportHandle?>(null)
    private var eventLoop: Job? = null
    internal var syncProbeJob: Job? = null
    internal var diagnosticsSampleJob: Job? = null
    internal var playbackRuntime: FfiListenerPlaybackHandle? = null
    internal var currentStreamId: StreamId? = null

    /**
     * The transport's own clock, read at [playbackRuntime]'s construction
     * (when its `nowMs()` was ~0), so it doubles as that transport-clock
     * reading's translation into the runtime's timeline. The transport
     * connects before any stream's runtime exists, so the two clocks have
     * different origins -- see [FfiListenerTransportEvent.SyncResponseReceived]'s
     * `receivedAtElapsedMs` doc comment and [translateToPumpClock].
     */
    internal var transportClockOriginMs: ULong? = null

    internal val _connectState = MutableStateFlow<ManualConnectUiState>(ManualConnectUiState.Idle)
    val connectState: StateFlow<ManualConnectUiState> = _connectState.asStateFlow()

    internal var trustedForFuture: Boolean = false
    internal var sessionId: SessionId? = null
    private var protocolVersion: Int = 0

    internal val logger = AppLogger("ManualListenerAudio")
    internal var lastReceivedSequence: Long? = null
    internal var receivedCount: Long = 0
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
            is FfiListenerTransportEvent.SyncResponseReceived -> handleSyncResponse(handle, event)
            is FfiListenerTransportEvent.AudioReceived -> handleAudioReceived(event)
        }
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
