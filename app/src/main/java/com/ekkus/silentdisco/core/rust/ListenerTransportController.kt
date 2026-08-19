package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportException
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportHandle
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
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
    private val handleRef = AtomicReference<FfiListenerTransportHandle?>(null)
    private var eventLoop: Job? = null
    private val eventChannel = Channel<FfiListenerTransportEvent>(Channel.UNLIMITED)

    override val events: Flow<FfiListenerTransportEvent> = eventChannel.receiveAsFlow()

    override suspend fun connect(
        scope: CoroutineScope,
        rawEndpoint: String,
        localDeviceId: String,
        localAddress: String,
    ) {
        withContext(Dispatchers.IO) {
            closeExistingHandle(reason = null)
            val handle = FfiListenerTransportHandle.connect(rawEndpoint, nowMs(), localDeviceId, localAddress)
            handleRef.set(handle)
            startEventLoop(scope, handle)
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

    override fun attachPlayback(playback: FfiListenerPlaybackHandle) {
        val handle = handleRef.get() ?: error("listener transport is not connected")
        handle.attachPlayback(playback)
    }

    override fun detachPlayback() {
        handleRef.get()?.detachPlayback()
    }

    override suspend fun disconnect(reason: String) {
        withContext(Dispatchers.IO) {
            closeExistingHandle(reason)
        }
    }

    override fun close() {
        eventLoop?.cancel()
        eventLoop = null
        closeExistingHandle(reason = null)
        eventChannel.close()
    }

    private fun startEventLoop(scope: CoroutineScope, handle: FfiListenerTransportHandle) {
        eventLoop?.cancel()
        eventLoop = scope.launch(Dispatchers.IO) {
            while (isActive && handleRef.get() === handle) {
                val event = try {
                    handle.pollEvent(POLL_TIMEOUT_MS)
                } catch (error: FfiListenerTransportException) {
                    eventChannel.trySend(
                        FfiListenerTransportEvent.ConnectionClosed(message = error.message),
                    )
                    break
                }
                if (event != null) {
                    eventChannel.trySend(event)
                }
            }
        }
    }

    private suspend fun <T> withHandle(action: (FfiListenerTransportHandle) -> T): T? =
        withContext(Dispatchers.IO) { handleRef.get()?.let(action) }

    /**
     * Always attempts every native cleanup operation and rethrows the first
     * failure after attaching later cleanup failures as suppressed exceptions.
     */
    private fun closeExistingHandle(reason: String?) {
        eventLoop?.cancel()
        eventLoop = null
        val handle = handleRef.getAndSet(null) ?: return
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
                recordFailure(error)
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
