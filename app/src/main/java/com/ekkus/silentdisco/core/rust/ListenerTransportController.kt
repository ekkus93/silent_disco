package com.ekkus.silentdisco.core.rust

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

/**
 * Android-facing wrapper around the shared Rust listener transport for one
 * BLE/Wi-Fi-Direct-discovered join at a time. Owns no domain policy: it only
 * opens the real transport, forwards the join request, and surfaces the
 * transport's own typed events as a [Flow] -- the caller
 * (`MainViewModelRustListener`) bridges those events into
 * `ListenerCoreController`. Deliberately separate from
 * [ManualListenerTransportController], which serves the manual-endpoint flow
 * and keeps its own [com.ekkus.silentdisco.core.model.ManualConnectUiState]
 * instead.
 */
class ListenerTransportController : AutoCloseable {
    private val handleRef = AtomicReference<FfiListenerTransportHandle?>(null)
    private var eventLoop: Job? = null
    private val eventChannel = Channel<FfiListenerTransportEvent>(Channel.UNLIMITED)

    val events: Flow<FfiListenerTransportEvent> = eventChannel.receiveAsFlow()

    suspend fun connect(
        scope: CoroutineScope,
        rawEndpoint: String,
        localDeviceId: String,
        localAddress: String,
    ) {
        withContext(Dispatchers.IO) {
            closeExistingHandle()
            val handle = FfiListenerTransportHandle.connect(rawEndpoint, nowMs(), localDeviceId, localAddress)
            handleRef.set(handle)
            startEventLoop(scope, handle)
        }
    }

    suspend fun sendJoinRequest(displayName: String, inviteCode: String?) =
        withHandle { it.sendJoinRequest(displayName, inviteCode) }

    suspend fun sendDisconnect(reason: String) = withHandle { it.sendDisconnect(reason) }

    override fun close() {
        eventLoop?.cancel()
        eventLoop = null
        closeExistingHandle()
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

    private fun closeExistingHandle() {
        handleRef.getAndSet(null)?.let { handle ->
            runCatching { handle.shutdown() }
            handle.close()
        }
    }

    private fun nowMs(): ULong = System.currentTimeMillis().toULong()
}
