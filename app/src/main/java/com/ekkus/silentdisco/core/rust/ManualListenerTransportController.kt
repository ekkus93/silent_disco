package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.model.ManualConnectUiState
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportException
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportHandle
import com.ekkus.silentdisco.core.uniffi.FfiManualHostEndpoint
import com.ekkus.silentdisco.core.uniffi.parseManualHostEndpoint
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

/**
 * Android-facing wrapper around the shared Rust listener transport for one
 * manual-endpoint connection attempt at a time. Owns no domain policy: it
 * only opens the real transport, forwards the caller's join request, and
 * projects the transport's own typed events into [ManualConnectUiState].
 */
class ManualListenerTransportController : AutoCloseable {
    private val handleRef = AtomicReference<FfiListenerTransportHandle?>(null)
    private var eventLoop: Job? = null

    private val _connectState = MutableStateFlow<ManualConnectUiState>(ManualConnectUiState.Idle)
    val connectState: StateFlow<ManualConnectUiState> = _connectState.asStateFlow()

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
        closeExistingHandle()
        _connectState.value = ManualConnectUiState.Idle
    }

    override fun close() {
        eventLoop?.cancel()
        eventLoop = null
        closeExistingHandle()
    }

    private fun startEventLoop(scope: CoroutineScope, handle: FfiListenerTransportHandle) {
        eventLoop?.cancel()
        eventLoop = scope.launch(Dispatchers.IO) {
            while (isActive && handleRef.get() === handle) {
                val event = try {
                    handle.pollEvent(POLL_TIMEOUT_MS)
                } catch (error: FfiListenerTransportException) {
                    // A transport that already connected can only report the connection
                    // ending, not a fresh configuration failure - always Disconnected here.
                    _connectState.value = ManualConnectUiState.Disconnected(error.message)
                    break
                }
                if (event != null) {
                    applyEvent(event)
                }
            }
        }
    }

    private fun applyEvent(event: FfiListenerTransportEvent) {
        _connectState.value = when (event) {
            is FfiListenerTransportEvent.Hello -> ManualConnectUiState.AwaitingApproval(
                hostName = event.hostName,
                sessionName = event.sessionName,
            )
            is FfiListenerTransportEvent.JoinApproved -> ManualConnectUiState.Approved(event.trustedForFuture)
            is FfiListenerTransportEvent.JoinRejected -> ManualConnectUiState.Rejected(event.reason)
            is FfiListenerTransportEvent.HostDisconnected -> ManualConnectUiState.Disconnected(event.reason)
            is FfiListenerTransportEvent.ConnectionClosed -> ManualConnectUiState.Disconnected(event.message)
            is FfiListenerTransportEvent.Rejected -> ManualConnectUiState.Failed(event.message)
            FfiListenerTransportEvent.StreamStarted,
            FfiListenerTransportEvent.Paused,
            FfiListenerTransportEvent.Stopped,
            -> _connectState.value
        }
    }

    private fun closeExistingHandle() {
        handleRef.getAndSet(null)?.let { handle ->
            runCatching { handle.shutdown() }
            handle.close()
        }
    }

    private fun nowMs(): ULong = System.currentTimeMillis().toULong()
}
