package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import com.ekkus.silentdisco.core.uniffi.FfiSyncConfidence
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportException
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportHandle
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private const val POLL_TIMEOUT_MS: ULong = 500uL

/** Narrow reusable listener-transport port used by the discovered-session effect runner. */
interface ListenerTransport : AutoCloseable {
    val events: Flow<FfiListenerTransportEvent>

    suspend fun connect(
        scope: CoroutineScope,
        rawEndpoint: String,
        localDeviceId: String,
        localAddress: String,
    )

    suspend fun sendJoinRequest(displayName: String, inviteCode: String?)
    suspend fun sendDisconnect(reason: String)
    suspend fun sendSyncRequest(correlationId: ULong, localSendElapsedMs: ULong)
    fun sendSynchronizationReport(
        confidence: FfiSyncConfidence,
        offsetMs: Double,
        roundTripMs: Double,
        driftPpm: Double,
    )
    fun attachPlayback(playback: FfiListenerPlaybackHandle)
    fun detachPlayback()

    /**
     * Tears down the current native connection while keeping [events] reusable
     * for a later discovered session.
     */
    suspend fun disconnect(reason: String)
}

/**
 * Android-facing wrapper around the shared Rust listener transport for one
 * discovered join at a time. Owns no domain policy: it only opens the real
 * transport, forwards control requests, and surfaces the transport's typed
 * events as a [Flow].
 */
class ListenerTransportController : ListenerTransport {
    private val handleGate = ListenerNativeHandleGate<FfiListenerTransportHandle>()
    private var eventLoop: Job? = null
    // A delayed collector must never see an approval/Stop from a retired socket.
    private val eventChannel = Channel<Pair<FfiListenerTransportHandle, FfiListenerTransportEvent>>(
        Channel.UNLIMITED,
    )

    override val events: Flow<FfiListenerTransportEvent> = eventChannel.receiveAsFlow()
        .filter { (handle, _) -> handleGate.currentHandle() === handle }
        .map { (_, event) -> event }

    override suspend fun connect(
        scope: CoroutineScope,
        rawEndpoint: String,
        localDeviceId: String,
        localAddress: String,
    ) {
        withContext(Dispatchers.IO) {
            handleGate.replace(
                retire = { retireHandle(it, reason = null) },
                open = {
                    FfiListenerTransportHandle.connect(
                        rawEndpoint, nowMs(), localDeviceId, localAddress,
                    )
                },
                start = { startEventLoop(scope, it) },
            )
        }
    }

    override suspend fun sendJoinRequest(displayName: String, inviteCode: String?) {
        withHandle { it.sendJoinRequest(displayName, inviteCode) }
    }

    override suspend fun sendDisconnect(reason: String) {
        withHandle { it.sendDisconnect(reason) }
    }

    override suspend fun sendSyncRequest(correlationId: ULong, localSendElapsedMs: ULong) {
        withHandle { it.sendSyncRequest(correlationId, localSendElapsedMs) }
    }

    override fun sendSynchronizationReport(
        confidence: FfiSyncConfidence,
        offsetMs: Double,
        roundTripMs: Double,
        driftPpm: Double,
    ) {
        handleGate.withCurrent {
            it.sendSynchronizationReport(confidence, offsetMs, roundTripMs, driftPpm)
        }
    }

    override fun attachPlayback(playback: FfiListenerPlaybackHandle) {
        handleGate.withCurrent { it.attachPlayback(playback) }
            ?: error("listener transport is not connected")
    }

    override fun detachPlayback() {
        handleGate.withCurrent { it.detachPlayback() }
    }

    override suspend fun disconnect(reason: String) {
        withContext(Dispatchers.IO) {
            handleGate.close { retireHandle(it, reason) }
        }
    }

    override fun close() {
        handleGate.close { retireHandle(it, reason = null) }
        eventChannel.close()
    }

    private fun startEventLoop(scope: CoroutineScope, handle: FfiListenerTransportHandle) {
        eventLoop?.cancel()
        eventLoop = scope.launch(Dispatchers.IO) {
            while (isActive && handleGate.currentHandle() === handle) {
                val event = try {
                    handle.pollEvent(POLL_TIMEOUT_MS)
                } catch (error: FfiListenerTransportException) {
                    // A host's explicit Disconnect is handled by the consumer,
                    // which closes this handle. The blocking poll can then
                    // unblock with a native channel-closed/Broken pipe error.
                    // That is cleanup fallout, not a second transport failure.
                    if (shouldForwardPolledListenerEvent(
                            loopActive = isActive,
                            handleIsCurrent = handleGate.currentHandle() === handle,
                        )
                    ) {
                        eventChannel.trySend(
                            handle to FfiListenerTransportEvent.ConnectionClosed(
                                message = polledConnectionClosedMessage(error),
                            ),
                        )
                    }
                    break
                }
                if (!shouldForwardPolledListenerEvent(
                        loopActive = isActive,
                        handleIsCurrent = handleGate.currentHandle() === handle,
                    )
                ) {
                    break
                }
                if (event != null) {
                    eventChannel.trySend(handle to event)
                }
            }
        }
    }

    private suspend fun <T> withHandle(action: (FfiListenerTransportHandle) -> T): T? {
        // Capture the session before dispatch: an operation queued behind
        // disconnect/rebind must not execute on a replacement native handle.
        val expected = handleGate.currentHandle() ?: return null
        return withContext(Dispatchers.IO) { handleGate.withExpected(expected, action) }
    }

    /**
     * Always attempts every native cleanup operation and rethrows the first
     * failure after attaching later cleanup failures as suppressed exceptions.
     */
    private fun retireHandle(handle: FfiListenerTransportHandle?, reason: String?) {
        eventLoop?.cancel()
        eventLoop = null
        handle ?: return
        var firstFailure: Throwable? = null

        fun recordFailure(error: Throwable) {
            val first = firstFailure
            if (first == null) {
                firstFailure = error
            } else {
                first.addSuppressed(error)
            }
        }

        if (reason != null) {
            try {
                handle.sendDisconnect(reason)
            } catch (error: FfiListenerTransportException) {
                if (!isPeerClosedDisconnectWriteFailure(error)) {
                    recordFailure(error)
                }
            }
        }
        try {
            handle.shutdown()
        } catch (error: FfiListenerTransportException) {
            recordFailure(error)
        }
        try {
            handle.close()
        } catch (error: RuntimeException) {
            recordFailure(error)
        }
        firstFailure?.let { throw it }
    }

    private fun nowMs(): ULong = System.currentTimeMillis().toULong()
}

internal fun shouldForwardPolledListenerEvent(
    loopActive: Boolean,
    handleIsCurrent: Boolean,
): Boolean = loopActive && handleIsCurrent

/** A typed Closed result from an active poll means the peer/runtime ended the connection. */
internal fun polledConnectionClosedMessage(error: FfiListenerTransportException): String? =
    if (error is FfiListenerTransportException.Closed) null else error.message

/** A peer that already closed its TCP stream cannot receive our final Leave frame. */
internal fun isPeerClosedDisconnectWriteFailure(error: FfiListenerTransportException): Boolean {
    if (error !is FfiListenerTransportException.Io) return false
    val message = error.message ?: return false
    val prefix = "control transport Delivery: failed to write control frame:"
    if (!message.startsWith(prefix, ignoreCase = true)) return false
    val detail = message.substring(prefix.length).lowercase()
    return listOf("broken pipe", "connection reset", "connection aborted").any(detail::contains)
}
