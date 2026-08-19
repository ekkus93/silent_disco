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
internal const val LISTENER_DISCOVERY_PROTOCOL_VERSION: Int = 2

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
    // mDNS is a standard-IP convenience layer and does not require the
    // Bluetooth/Wi-Fi-Direct nearby-device permission set. Start it first so
    // a desktop host remains discoverable even when the user declines those
    // optional radios; manual endpoint entry remains available regardless.
    val mdnsResult = mdnsService.startDiscovery()
    if (!hasListenerTransportPermissions()) {
        if (mdnsResult.started) {
            controller.platformOperationSucceeded(effect.operationId, FfiPlatformCompletion.DiscoveryStarted)
        } else {
            controller.platformOperationFailed(
                effect.operationId,
                mdnsResult.message ?: "Missing nearby connectivity permissions and mDNS discovery is unavailable",
                true,
            )
        }
        return
    }

    val bleResult = bleService.startScanning()
    if (bleResult.started) {
        wifiDirectService.discoverPeers()
    }
    if (mdnsResult.started || bleResult.started) {
        controller.platformOperationSucceeded(effect.operationId, FfiPlatformCompletion.DiscoveryStarted)
    } else {
        controller.platformOperationFailed(
            effect.operationId,
            bleResult.message ?: mdnsResult.message ?: "No discovery backend could start",
            true,
        )
    }
}

internal fun MainViewModel.stopRustListenerDiscovery(
    controller: ListenerCoreController,
    effect: FfiPlatformEffect.StopDiscovery,
) {
    var firstFailure: RuntimeException? = null
    fun attempt(action: () -> Unit) {
        try {
            action()
        } catch (error: RuntimeException) {
            val first = firstFailure
            if (first == null) firstFailure = error else first.addSuppressed(error)
        }
    }
    attempt(mdnsService::stopDiscovery)
    attempt(bleService::stopScanning)
    attempt(wifiDirectService::cancelDiscovery)
    val failure = firstFailure
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
    val address = effect.address
    val controlPort = effect.controlPort
    val syncPort = effect.syncPort
    val audioPort = effect.audioPort
    if (address != null && controlPort != null && syncPort != null && audioPort != null) {
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
            protocolVersion = mdnsService.discoveredSessions.value
                .firstOrNull { it.sessionId == effect.sessionId }
                ?.protocolVersion
                ?: LISTENER_DISCOVERY_PROTOCOL_VERSION,
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
    wifiDirectService.connectToSession(session)
}

internal fun MainViewModel.releaseRustListenerNetwork(
    controller: ListenerCoreController,
    effect: FfiPlatformEffect.ReleaseNetwork,
) {
    pendingEstablishNetworkOperationId = null
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
    protocolVersion: Int = LISTENER_DISCOVERY_PROTOCOL_VERSION,
) {
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
            )
            listenerTransportController.sendJoinRequest(LISTENER_DISPLAY_NAME, inviteCode)
        } catch (error: FfiListenerTransportException) {
            controller.platformOperationFailed(
                operationId,
                error.message ?: "listener transport connection failed",
                true,
            )
            return@launch
        } catch (error: RuntimeException) {
            controller.platformOperationFailed(
                operationId,
                error.message ?: "listener transport connection failed",
                true,
            )
            return@launch
        }
        ensureListenerTransportEventLoop()
        controller.platformOperationSucceeded(
            operationId,
            FfiPlatformCompletion.NetworkEndpointReady(address, controlPort, syncPort, audioPort),
        )
    }
}

/**
 * Called from the existing Wi-Fi Direct snapshot collector once the
 * transport reaches CONNECTED -- opens the real Rust listener transport
 * against the endpoint Wi-Fi Direct actually resolved, sends the join
 * request over it, and only then completes the pending EstablishNetwork
 * operation. Mirrors `completeRustHostAdvertising`'s async-completion shape
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
