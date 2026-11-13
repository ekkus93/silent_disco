package com.ekkus.silentdisco.app

import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.rust.ListenerCoreController
import com.ekkus.silentdisco.core.transport.TransportPorts
import com.ekkus.silentdisco.core.transport.TransportSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportException
import com.ekkus.silentdisco.core.uniffi.FfiPlatformCompletion
import com.ekkus.silentdisco.core.uniffi.FfiPlatformEffect
import kotlinx.coroutines.launch
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

private const val LOCAL_LISTENER_BIND_ADDRESS = "0.0.0.0"
private const val LISTENER_DISPLAY_NAME = "This Android Listener"
/** Protocol version advertised by this app's BLE and desktop-mDNS discovery layers. */
internal const val LISTENER_DISCOVERY_PROTOCOL_VERSION: Int = 3

/**
 * Endpoint version this app joins (phase 4, owner decision 2026-11-13):
 * secure transport v4 only. Used where the host's version is not advertised
 * (an Android host found over Wi-Fi Direct), since every host this app can
 * join is v4. The Rust transport refuses anything else before connecting.
 */
internal const val SECURE_TRANSPORT_ENDPOINT_VERSION: Int = 4

@Serializable
private data class ManualHostEndpointPayload(
    val hostAddress: String,
    val controlPort: Int,
    val syncPort: Int,
    val audioPort: Int,
    val sessionId: String,
    val protocolVersion: Int,
    val inviteCodeRequired: Boolean,
    val expiresAtMs: String? = null,
)

internal fun MainViewModel.startRustListenerDiscovery(
    controller: ListenerCoreController,
    effect: FfiPlatformEffect.StartDiscovery,
) {
    when (val failure = discovery.start(hasNearbyPermissions = hasListenerTransportPermissions())) {
        null -> controller.platformOperationSucceeded(effect.operationId, FfiPlatformCompletion.DiscoveryStarted)
        else -> controller.platformOperationFailed(effect.operationId, failure, true)
    }
}

internal fun MainViewModel.stopRustListenerDiscovery(
    controller: ListenerCoreController,
    effect: FfiPlatformEffect.StopDiscovery,
) {
    val failure = discovery.stop()
    if (failure == null) {
        controller.platformOperationSucceeded(effect.operationId, FfiPlatformCompletion.DiscoveryStopped)
    } else {
        controller.platformOperationFailed(
            effect.operationId,
            failure.message ?: "Discovery cleanup failed",
            true,
        )
    }
}

internal fun MainViewModel.establishRustListenerNetwork(
    controller: ListenerCoreController,
    effect: FfiPlatformEffect.EstablishNetwork,
) {
    listenerTransportConnected = false
    val address = effect.address
    val controlPort = effect.controlPort
    val syncPort = effect.syncPort
    val audioPort = effect.audioPort
    if (address != null && controlPort != null && syncPort != null && audioPort != null) {
        listenerTransportRidesWifiDirect = false
        val session = _uiState.value.discoveredSessions.firstOrNull { it.id == effect.sessionId }
        if (session == null) {
            controller.platformOperationFailed(
                effect.operationId,
                "Selected mDNS session disappeared before establishment",
                false,
            )
            return
        }
        connectRustListenerEndpoint(
            controller = controller,
            operationId = effect.operationId,
            session = session,
            address = address,
            controlPort = controlPort,
            syncPort = syncPort,
            audioPort = audioPort,
            protocolVersion = discovery.mdnsSessions.value
                .firstOrNull { it.sessionId == effect.sessionId }
                ?.protocolVersion
                ?: SECURE_TRANSPORT_ENDPOINT_VERSION,
        )
        return
    }
    val session = _uiState.value.discoveredSessions.firstOrNull { it.id == effect.sessionId }
    if (session == null) {
        controller.platformOperationFailed(
            effect.operationId,
            "Selected session disappeared before establishment",
            false,
        )
        return
    }
    pendingEstablishNetworkOperationId = effect.operationId
    listenerTransportRidesWifiDirect = true
    wifiDirectService.connectToSession(session)
}

internal fun MainViewModel.releaseRustListenerNetwork(
    controller: ListenerCoreController,
    effect: FfiPlatformEffect.ReleaseNetwork,
) {
    pendingEstablishNetworkOperationId = null
    listenerTransportRidesWifiDirect = false
    listenerTransportConnected = false
    viewModelScope.launch {
        var firstFailure: Throwable? = null
        try {
            listenerTransportController.disconnect("Listener released network")
        } catch (error: FfiListenerTransportException) {
            firstFailure = error
        } catch (error: RuntimeException) {
            firstFailure = error
        }
        try {
            wifiDirectService.stop()
        } catch (error: RuntimeException) {
            val first = firstFailure
            if (first == null) firstFailure = error else first.addSuppressed(error)
        }
        val failure = firstFailure
        if (failure == null) {
            controller.platformOperationSucceeded(effect.operationId, FfiPlatformCompletion.NetworkReleased)
        } else {
            controller.platformOperationFailed(
                effect.operationId,
                failure.message ?: "Listener network cleanup failed",
                true,
            )
        }
    }
}

private fun MainViewModel.connectRustListenerEndpoint(
    controller: ListenerCoreController,
    operationId: String,
    session: SessionInfo,
    address: String,
    controlPort: UShort,
    syncPort: UShort,
    audioPort: UShort,
    protocolVersion: Int = SECURE_TRANSPORT_ENDPOINT_VERSION,
) {
    logger.i(
        "listener.endpoint_connect",
        "Opening listener endpoint for session ${session.id} (Wi-Fi Direct=$listenerTransportRidesWifiDirect)",
    )
    val rawEndpoint = Json.encodeToString(
        ManualHostEndpointPayload.serializer(),
        ManualHostEndpointPayload(
            hostAddress = address,
            controlPort = controlPort.toInt(),
            syncPort = syncPort.toInt(),
            audioPort = audioPort.toInt(),
            sessionId = session.id,
            protocolVersion = protocolVersion,
            inviteCodeRequired = session.inviteCodeRequired,
        ),
    )
    val inviteCode = _uiState.value.connectionProgress.inviteCode.ifBlank { null }
    viewModelScope.launch {
        try {
            listenerTransportController.connect(
                viewModelScope,
                rawEndpoint,
                localListenerDeviceId,
                LOCAL_LISTENER_BIND_ADDRESS,
                invitedHostTlsKeys[session.id],
            )
        } catch (error: FfiListenerTransportException) {
            logger.e(
                "listener.endpoint_connect_failed",
                error.message ?: "listener transport connection failed",
                error,
            )
            controller.platformOperationFailed(
                operationId,
                error.message ?: "listener transport connection failed",
                true,
            )
            return@launch
        } catch (error: RuntimeException) {
            logger.e(
                "listener.endpoint_connect_failed",
                error.message ?: "listener transport connection failed",
                error,
            )
            controller.platformOperationFailed(
                operationId,
                error.message ?: "listener transport connection failed",
                true,
            )
            return@launch
        }
        listenerTransportConnected = true
        logger.i(
            "listener.endpoint_connected",
            "Listener endpoint connected; reporting readiness to Rust",
        )
        // Report the endpoint ready before soliciting any reply. The core only
        // accepts the host's "awaiting approval" once it is `Connecting`, and it
        // applies submissions in order, so this must be queued before the join
        // request can provoke a reply (Block 52.2). The transport's event
        // channel is buffered, so starting the forwarding loop afterwards
        // cannot lose that reply.
        controller.platformOperationSucceeded(
            operationId,
            FfiPlatformCompletion.NetworkEndpointReady(address, controlPort, syncPort, audioPort),
        )
        ensureListenerTransportEventLoop()
        try {
            listenerTransportController.sendJoinRequest(LISTENER_DISPLAY_NAME, inviteCode)
            logger.i(
                "listener.join_request_sent",
                "Join request sent; waiting for the host decision",
            )
        } catch (error: kotlinx.coroutines.CancellationException) {
            throw error
        } catch (error: Exception) {
            // NetworkEndpointReady was already reported to the actor. Do not
            // leave a live socket or a misleading CONNECTED flag after a
            // failed/stale join send; the actor gets an explicit retryable
            // failure instead of an indefinite approval spinner.
            listenerTransportConnected = false
            runCatching { listenerTransportController.disconnect("Join request failed") }
                .exceptionOrNull()
                ?.let { cleanup -> if (cleanup !== error) error.addSuppressed(cleanup) }
            logger.e(
                "listener.join_request_failed",
                error.message ?: "join request failed",
                error,
            )
            controller.transportFailed(error.message ?: "join request failed", true)
        }
    }
}

/**
 * Called from the existing Wi-Fi Direct snapshot collector once the
 * transport reaches CONNECTED -- opens the real Rust listener transport
 * against the endpoint Wi-Fi Direct actually resolved, completes the pending
 * EstablishNetwork operation, and only then sends the join request, so the
 * host's reply cannot reach the core before it is `Connecting`. Mirrors `completeRustHostAdvertising`'s async-completion shape
 * on the host side.
 */
internal fun MainViewModel.completeRustListenerNetworkEstablishment(
    snapshot: TransportSnapshot,
) {
    val operationId = pendingEstablishNetworkOperationId ?: return
    if (snapshot.state != TransportConnectionState.CONNECTED) return
    val controller = listenerCoreController ?: return
    val address = snapshot.hostAddressHint
    if (address == null) {
        pendingEstablishNetworkOperationId = null
        controller.platformOperationFailed(
            operationId,
            "Wi-Fi Direct reported connected without a resolved host address",
            true,
        )
        return
    }
    val session = _uiState.value.selectedSession
    if (session == null) {
        pendingEstablishNetworkOperationId = null
        controller.platformOperationFailed(operationId, "Selected session disappeared before establishment", false)
        return
    }
    pendingEstablishNetworkOperationId = null
    val ports = TransportPorts()
    connectRustListenerEndpoint(
        controller = controller,
        operationId = operationId,
        session = session,
        address = address,
        controlPort = ports.control.toUShort(),
        syncPort = ports.sync.toUShort(),
        audioPort = ports.audio.toUShort(),
    )
}
