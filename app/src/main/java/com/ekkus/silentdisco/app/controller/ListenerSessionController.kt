package com.ekkus.silentdisco.app.controller

import com.ekkus.silentdisco.core.logging.AppLogger
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.rust.ListenerCoreController
import com.ekkus.silentdisco.core.rust.ListenerTransport
import com.ekkus.silentdisco.core.transport.SessionTransport
import com.ekkus.silentdisco.core.transport.TransportPorts
import com.ekkus.silentdisco.core.transport.TransportSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiCoreNotification
import com.ekkus.silentdisco.core.uniffi.FfiCoreSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportException
import com.ekkus.silentdisco.core.uniffi.FfiPlatformCompletion
import com.ekkus.silentdisco.core.uniffi.FfiPlatformEffect
import com.ekkus.silentdisco.core.uniffi.FfiSyncConfidence
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.launch
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

private const val LOCAL_LISTENER_BIND_ADDRESS = "0.0.0.0"
private const val LISTENER_DISPLAY_NAME = "This Android Listener"

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

/** What the listener session reads from, and hands back to, the coordinator. */
internal interface ListenerSessionCallbacks {
    /** A session the core discovered, by id, as the UI currently lists it. */
    fun discoveredSession(sessionId: String): SessionInfo?

    /** The session the listener selected, if any. */
    val selectedSession: SessionInfo?

    /** The invite code the user entered, if any. */
    val inviteCode: String?

    /** Whether the listener may use the nearby-device radios. */
    fun hasNearbyPermissions(): Boolean

    /** A core snapshot to map into the UI. */
    fun onSnapshot(snapshot: FfiCoreSnapshot)

    /** One event from the listener transport, for the coordinator to route. */
    fun onTransportEvent(core: ListenerCoreController, event: FfiListenerTransportEvent)
}

/**
 * The listener's session (God-file refactor spec §4): the Rust listener core
 * and the platform work it asks for, network establishment (a discovered
 * endpoint, or a Wi-Fi Direct group once it connects), the join request,
 * network release, the transport event loop and the TLS keys pinned from
 * verified invitations.
 *
 * Rust owns join legality: this only performs the platform operations the
 * core requests and reports their outcome. It never writes UI state.
 */
internal class ListenerSessionController(
    parentScope: CoroutineScope,
    private val transport: ListenerTransport,
    private val wifiDirect: SessionTransport,
    private val discovery: DiscoveryController,
    private val coreFactory: (String) -> ListenerCoreController,
    private val deviceId: () -> String,
    private val logger: AppLogger,
    private val callbacks: ListenerSessionCallbacks,
) : ViewModelController(parentScope) {
    /** The Rust listener core, once the listener role starts. */
    var core: ListenerCoreController? = null

    /** This device's stable listener id, read once. */
    val localDeviceId: String by lazy(deviceId)

    /** The core's EstablishNetwork waiting for Wi-Fi Direct to connect. */
    var pendingEstablishOperationId: String? = null

    /** True once the listener endpoint is connected, until it is released or fails. */
    var transportConnected: Boolean = false

    /**
     * Whether the joined session's transport runs over a Wi-Fi Direct group.
     * Only then is Wi-Fi Direct's own connection state evidence about the
     * session; a desktop host joined via mDNS or a manual endpoint never uses
     * it, and its disconnects arrive as Rust transport events instead.
     */
    var ridesWifiDirect: Boolean = false

    val hasPendingWifiDirectJoin: Boolean
        get() = ridesWifiDirect || pendingEstablishOperationId != null

    private var eventLoopStarted = false

    /** TLS keys signed into verified invitations, by session id; a join of that session pins to it. */
    private val invitedHostTlsKeys = mutableMapOf<String, ByteArray>()

    fun rememberInvitedHostTlsKey(sessionId: String, key: ByteArray) {
        invitedHostTlsKeys[sessionId] = key
    }

    fun invitedHostTlsKey(sessionId: String): ByteArray? = invitedHostTlsKeys[sessionId]

    /** Creates the core on first use and starts mapping its snapshots and running its effects. */
    fun ensureCore(): ListenerCoreController {
        core?.let { return it }
        val created = coreFactory(localDeviceId)
        core = created
        scope.launch { created.snapshots.filterNotNull().collect(callbacks::onSnapshot) }
        scope.launch { created.notifications.collect { execute(created, it) } }
        return created
    }

    private fun execute(core: ListenerCoreController, notification: FfiCoreNotification) {
        when (notification) {
            is FfiCoreNotification.Snapshot -> callbacks.onSnapshot(notification.snapshot)
            is FfiCoreNotification.PlatformEffect -> executePlatformEffect(core, notification.effect)
            is FfiCoreNotification.TransportEffect -> Unit // the listener role never emits transport effects
            is FfiCoreNotification.StorageEffect -> Unit // the listener role does not persist state yet
            is FfiCoreNotification.Error -> logger.e("rust.listener", notification.error.message)
            is FfiCoreNotification.Diagnostic -> logger.i(
                "rust.listener.${notification.diagnostic.name}",
                notification.diagnostic.fields.joinToString(),
            )
        }
    }

    private fun executePlatformEffect(core: ListenerCoreController, effect: FfiPlatformEffect) {
        when (effect) {
            is FfiPlatformEffect.StartDiscovery ->
                when (val failure = discovery.start(callbacks.hasNearbyPermissions())) {
                    null -> core.platformOperationSucceeded(effect.operationId, FfiPlatformCompletion.DiscoveryStarted)
                    else -> core.platformOperationFailed(effect.operationId, failure, true)
                }
            is FfiPlatformEffect.StopDiscovery -> {
                val failure = discovery.stop()
                if (failure == null) {
                    core.platformOperationSucceeded(effect.operationId, FfiPlatformCompletion.DiscoveryStopped)
                } else {
                    core.platformOperationFailed(effect.operationId, failure.message ?: "Discovery cleanup failed", true)
                }
            }
            is FfiPlatformEffect.EstablishNetwork -> establishNetwork(core, effect)
            is FfiPlatformEffect.ReleaseNetwork -> releaseNetwork(core, effect)
            is FfiPlatformEffect.StartAdvertising -> unsupported(core, effect.operationId)
            is FfiPlatformEffect.StopAdvertising -> unsupported(core, effect.operationId)
            is FfiPlatformEffect.RequestCapabilities -> unsupported(core, effect.operationId)
            is FfiPlatformEffect.PrepareAudioSource -> unsupported(core, effect.operationId)
            is FfiPlatformEffect.StartAudioOutput -> unsupported(core, effect.operationId)
            is FfiPlatformEffect.StopAudioOutput -> unsupported(core, effect.operationId)
            is FfiPlatformEffect.ShareDiagnostics -> unsupported(core, effect.operationId)
        }
    }

    private fun unsupported(core: ListenerCoreController, operationId: String) =
        core.platformOperationFailed(operationId, "Platform effect is outside Android listener Block 13", false)

    private fun establishNetwork(core: ListenerCoreController, effect: FfiPlatformEffect.EstablishNetwork) {
        transportConnected = false
        val address = effect.address
        val controlPort = effect.controlPort
        val syncPort = effect.syncPort
        val audioPort = effect.audioPort
        if (address != null && controlPort != null && syncPort != null && audioPort != null) {
            ridesWifiDirect = false
            val session = callbacks.discoveredSession(effect.sessionId) ?: run {
                core.platformOperationFailed(
                    effect.operationId,
                    "Selected mDNS session disappeared before establishment",
                    false,
                )
                return
            }
            connectEndpoint(
                core = core,
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
        val session = callbacks.discoveredSession(effect.sessionId) ?: run {
            core.platformOperationFailed(effect.operationId, "Selected session disappeared before establishment", false)
            return
        }
        pendingEstablishOperationId = effect.operationId
        ridesWifiDirect = true
        wifiDirect.connectToSession(session)
    }

    private fun releaseNetwork(core: ListenerCoreController, effect: FfiPlatformEffect.ReleaseNetwork) {
        pendingEstablishOperationId = null
        ridesWifiDirect = false
        transportConnected = false
        scope.launch {
            var firstFailure: Throwable? = null
            try {
                transport.disconnect("Listener released network")
            } catch (error: FfiListenerTransportException) {
                firstFailure = error
            } catch (error: RuntimeException) {
                firstFailure = error
            }
            try {
                wifiDirect.stop()
            } catch (error: RuntimeException) {
                val first = firstFailure
                if (first == null) firstFailure = error else first.addSuppressed(error)
            }
            val failure = firstFailure
            if (failure == null) {
                core.platformOperationSucceeded(effect.operationId, FfiPlatformCompletion.NetworkReleased)
            } else {
                core.platformOperationFailed(
                    effect.operationId,
                    failure.message ?: "Listener network cleanup failed",
                    true,
                )
            }
        }
    }

    /**
     * Called once Wi-Fi Direct reaches CONNECTED: opens the Rust listener
     * transport against the endpoint Wi-Fi Direct actually resolved,
     * completes the pending EstablishNetwork operation, and only then sends
     * the join request, so the host's reply cannot reach the core before it
     * is `Connecting`.
     */
    fun completeWifiDirectEstablishment(snapshot: TransportSnapshot) {
        val operationId = pendingEstablishOperationId ?: return
        if (snapshot.state != TransportConnectionState.CONNECTED) return
        val core = core ?: return
        val address = snapshot.hostAddressHint
        if (address == null) {
            pendingEstablishOperationId = null
            core.platformOperationFailed(
                operationId,
                "Wi-Fi Direct reported connected without a resolved host address",
                true,
            )
            return
        }
        val session = callbacks.selectedSession
        if (session == null) {
            pendingEstablishOperationId = null
            core.platformOperationFailed(operationId, "Selected session disappeared before establishment", false)
            return
        }
        pendingEstablishOperationId = null
        val ports = TransportPorts()
        connectEndpoint(
            core = core,
            operationId = operationId,
            session = session,
            address = address,
            controlPort = ports.control.toUShort(),
            syncPort = ports.sync.toUShort(),
            audioPort = ports.audio.toUShort(),
        )
    }

    private fun connectEndpoint(
        core: ListenerCoreController,
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
            "Opening listener endpoint for session ${session.id} (Wi-Fi Direct=$ridesWifiDirect)",
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
        val inviteCode = callbacks.inviteCode
        scope.launch {
            try {
                transport.connect(
                    scope,
                    rawEndpoint,
                    localDeviceId,
                    LOCAL_LISTENER_BIND_ADDRESS,
                    invitedHostTlsKeys[session.id],
                )
            } catch (error: FfiListenerTransportException) {
                reportConnectFailure(core, operationId, error)
                return@launch
            } catch (error: RuntimeException) {
                reportConnectFailure(core, operationId, error)
                return@launch
            }
            transportConnected = true
            logger.i("listener.endpoint_connected", "Listener endpoint connected; reporting readiness to Rust")
            // Report the endpoint ready before soliciting any reply. The core only
            // accepts the host's "awaiting approval" once it is `Connecting`, and it
            // applies submissions in order, so this must be queued before the join
            // request can provoke a reply (Block 52.2). The transport's event
            // channel is buffered, so starting the forwarding loop afterwards
            // cannot lose that reply.
            core.platformOperationSucceeded(
                operationId,
                FfiPlatformCompletion.NetworkEndpointReady(address, controlPort, syncPort, audioPort),
            )
            ensureEventLoop()
            try {
                transport.sendJoinRequest(LISTENER_DISPLAY_NAME, inviteCode)
                logger.i("listener.join_request_sent", "Join request sent; waiting for the host decision")
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                // NetworkEndpointReady was already reported to the actor. Do not
                // leave a live socket or a misleading CONNECTED flag after a
                // failed/stale join send; the actor gets an explicit retryable
                // failure instead of an indefinite approval spinner.
                transportConnected = false
                runCatching { transport.disconnect("Join request failed") }
                    .exceptionOrNull()
                    ?.let { cleanup -> if (cleanup !== error) error.addSuppressed(cleanup) }
                logger.e("listener.join_request_failed", error.message ?: "join request failed", error)
                core.transportFailed(error.message ?: "join request failed", true)
            }
        }
    }

    private fun reportConnectFailure(core: ListenerCoreController, operationId: String, error: Exception) {
        val message = error.message ?: "listener transport connection failed"
        logger.e("listener.endpoint_connect_failed", message, error)
        core.platformOperationFailed(operationId, message, true)
    }

    /** Starts, once, the loop that hands every transport event to the coordinator. */
    fun ensureEventLoop() {
        if (eventLoopStarted) return
        eventLoopStarted = true
        scope.launch(Dispatchers.IO) {
            transport.collectCurrentEvents { event ->
                val current = core ?: return@collectCurrentEvents
                callbacks.onTransportEvent(current, event)
            }
        }
    }

    /** Sends the listener's sync quality to the host; a failed send is only logged, as before. */
    fun reportSyncToHost(confidence: FfiSyncConfidence, offsetMs: Double, roundTripMs: Double, driftPpm: Double) {
        try {
            transport.sendSynchronizationReport(confidence, offsetMs, roundTripMs, driftPpm)
        } catch (error: RuntimeException) {
            logger.w("sync.report_send_failed", error.message ?: "synchronization report send failed")
        }
    }
}
