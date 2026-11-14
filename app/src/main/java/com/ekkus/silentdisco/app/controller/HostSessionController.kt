package com.ekkus.silentdisco.app.controller

import android.os.SystemClock
import com.ekkus.silentdisco.app.HostRecoveryDispatch
import com.ekkus.silentdisco.app.PendingHostJoinRoutes
import com.ekkus.silentdisco.app.newHostJoinRequestId
import com.ekkus.silentdisco.app.toFfiDeliveryReport
import com.ekkus.silentdisco.core.logging.AppLogger
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.rust.HostCoreController
import com.ekkus.silentdisco.core.rust.HostTransportController
import com.ekkus.silentdisco.core.rust.LEGACY_V3_DURABLE_TRUST_ERROR
import com.ekkus.silentdisco.core.rust.RustStoredTuningSettings
import com.ekkus.silentdisco.core.transport.TransportPorts
import com.ekkus.silentdisco.core.transport.TransportSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiCoreNotification
import com.ekkus.silentdisco.core.uniffi.FfiCoreSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiDeliveryReport
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportHandle
import com.ekkus.silentdisco.core.uniffi.FfiJoinRequestInput
import com.ekkus.silentdisco.core.uniffi.FfiListenerSummary
import com.ekkus.silentdisco.core.uniffi.FfiPlatformCompletion
import com.ekkus.silentdisco.core.uniffi.FfiPlatformEffect
import com.ekkus.silentdisco.core.uniffi.FfiStorageEffect
import com.ekkus.silentdisco.core.uniffi.FfiTransportEffect
import com.ekkus.silentdisco.core.uniffi.FfiTransportState
import com.ekkus.silentdisco.core.uniffi.FfiTrustState
import com.ekkus.silentdisco.core.uniffi.FfiTuningSettings
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.launch

/**
 * Binding the host transport needs the Wi-Fi Direct group's own address,
 * which the OS can assign fractionally after the group reports ADVERTISING.
 * A few short retries absorb that race (seen on a physical device) without
 * masking a genuinely permanent bind failure such as a port in use.
 */
private const val HOST_BIND_RETRY_ATTEMPTS = 5
private const val HOST_BIND_RETRY_DELAY_MS = 200L

/** What the host session reads from, and hands back to, the coordinator. */
internal interface HostSessionCallbacks {
    /** A core snapshot to map into the UI. */
    fun onSnapshot(snapshot: FfiCoreSnapshot)

    /** A platform effect (advertising start and stop, unsupported effects). */
    suspend fun executePlatformEffect(core: HostCoreController, effect: FfiPlatformEffect)

    /** The core reported an error to show. */
    fun onCoreError(message: String)

    /** Executing [notification] threw; complete it as failed and clean up. */
    fun recoverNotificationFailure(core: HostCoreController, notification: FfiCoreNotification, error: Exception)

    /** A host command failed; show it. */
    fun commandFailed(action: String, error: Throwable)

    /** The name a pending join request gave, if it is still listed. */
    fun pendingJoinDisplayName(listenerId: String): String?

    /** Persists tuning settings the core asked to save. */
    suspend fun saveTuning(settings: RustStoredTuningSettings)
}

/**
 * The host's session (God-file refactor spec §4): the Rust host core and its
 * notification loop, the host transport event loop, projecting transport
 * events into the core (join routes, clock-sync replies, resync replays),
 * executing the core's transport and storage effects, and completing
 * advertising once the transport binds.
 *
 * Rust owns admission: this only performs what the core asks and reports the
 * outcome. Every send is bound to the transport generation it came from, so a
 * late event from a retired host session cannot reach a rebound one.
 */
internal class HostSessionController(
    parentScope: CoroutineScope,
    private val transport: HostTransportController,
    private val coreFactory: (String) -> HostCoreController,
    private val deviceId: () -> String,
    private val logger: AppLogger,
    private val callbacks: HostSessionCallbacks,
    private val ioDispatcher: CoroutineDispatcher = Dispatchers.IO,
) : ViewModelController(parentScope) {
    @Volatile var core: HostCoreController? = null

    /** The session being hosted, from StartAdvertising until it stops. */
    @Volatile var sessionId: SessionId? = null

    /** The core's StartAdvertising, waiting for the transport to bind. */
    @Volatile var pendingStartAdvertisingOperationId: String? = null

    /** Suppresses duplicate Wi-Fi callbacks for the same in-flight bind operation. */
    private var bindingOperationId: String? = null

    private var eventLoopStarted = false

    /**
     * A listener's sync/audio UDP ports, from its `JoinRequestReceived`
     * event, kept only until the join is approved or otherwise resolved --
     * `authorize_listener` needs them, but nothing else does, so they never
     * belong in Rust-owned join-request state.
     */
    val joinRoutes = PendingHostJoinRoutes<FfiHostTransportHandle>()

    val recoveryDispatch = HostRecoveryDispatch()

    /** Creates the core on first use and starts its notification and transport event loops. */
    fun ensureCore(): HostCoreController {
        core?.let { return it }
        val created = coreFactory(deviceId())
        core = created
        scope.launch { created.snapshots.filterNotNull().collect(callbacks::onSnapshot) }
        scope.launch {
            created.notifications.collect { notification ->
                try {
                    execute(created, notification)
                } catch (error: CancellationException) {
                    throw error
                } catch (error: Exception) {
                    callbacks.recoverNotificationFailure(created, notification, error)
                }
            }
        }
        ensureEventLoop(created)
        return created
    }

    private suspend fun execute(core: HostCoreController, notification: FfiCoreNotification) {
        when (notification) {
            is FfiCoreNotification.Snapshot -> callbacks.onSnapshot(notification.snapshot)
            is FfiCoreNotification.PlatformEffect -> callbacks.executePlatformEffect(core, notification.effect)
            is FfiCoreNotification.TransportEffect -> executeTransportEffect(core, notification.effect)
            is FfiCoreNotification.StorageEffect -> executeStorageEffect(core, notification.effect)
            is FfiCoreNotification.Error -> {
                logger.e("rust.host", notification.error.message)
                callbacks.onCoreError(notification.error.message)
            }
            is FfiCoreNotification.Diagnostic ->
                logger.i("rust.host.${notification.diagnostic.name}", notification.diagnostic.fields.joinToString())
        }
    }

    private fun ensureEventLoop(core: HostCoreController) {
        if (eventLoopStarted) return
        eventLoopStarted = true
        scope.launch {
            transport.eventsWithGeneration.collect { source ->
                // Rebind can retire the socket after dequeue but before actor
                // projection. Never submit a stale join/disconnect to the new host.
                if (!transport.isCurrentGeneration(source.handle)) return@collect
                handleTransportEvent(core, source.event, source.handle)
            }
        }
    }

    /**
     * Completes a pending `StartAdvertising` once the Wi-Fi Direct group
     * forms and its own address is known: binding the host transport needs a
     * real local address, which is not available when the group is only
     * requested.
     */
    suspend fun completeAdvertising(snapshot: TransportSnapshot) {
        val operationId = pendingStartAdvertisingOperationId ?: return
        if (snapshot.state != TransportConnectionState.ADVERTISING) return
        val address = snapshot.hostAddressHint ?: return
        val controller = core ?: return
        val session = sessionId?.value ?: return
        // Wi-Fi Direct can report the same group address more than once while
        // the native bind is suspended. Never launch duplicate binds for one op.
        if (bindingOperationId == operationId) return
        bindingOperationId = operationId
        val stillRequested = {
            pendingStartAdvertisingOperationId == operationId &&
                sessionId?.value == session &&
                core === controller
        }
        try {
            val ports = TransportPorts()
            var lastError: Throwable? = null
            for (attempt in 1..HOST_BIND_RETRY_ATTEMPTS) {
                if (!stillRequested()) return
                val result = runCatching {
                    transport.bindIfRequested(
                        scope = scope,
                        localAddress = address,
                        controlPort = ports.control,
                        syncPort = ports.sync,
                        audioPort = ports.audio,
                        sessionId = session,
                        stillRequested = stillRequested,
                    )
                }
                // Stop or a replacement session can arrive while the IO bind is
                // queued. Never acknowledge the retired operation to the actor.
                if (!stillRequested()) return
                result.onSuccess { bound ->
                    if (!bound) return
                    pendingStartAdvertisingOperationId = null
                    joinRoutes.clear()
                    controller.transportStateChanged(FfiTransportState.ADVERTISING)
                    controller.platformOperationSucceeded(operationId, FfiPlatformCompletion.AdvertisingStarted)
                    return
                }
                lastError = result.exceptionOrNull()
                if (lastError is CancellationException) throw lastError
                logger.w(
                    "rust.host",
                    "Host transport bind attempt $attempt/$HOST_BIND_RETRY_ATTEMPTS at $address failed: ${lastError?.message}",
                )
                if (attempt < HOST_BIND_RETRY_ATTEMPTS && stillRequested()) {
                    delay(HOST_BIND_RETRY_DELAY_MS)
                }
            }
            if (stillRequested()) {
                pendingStartAdvertisingOperationId = null
                controller.platformOperationFailed(
                    operationId,
                    lastError?.message ?: "Failed to bind host transport",
                    true,
                )
            }
        } finally {
            if (bindingOperationId == operationId) {
                bindingOperationId = null
            }
        }
    }

    /** Native events are projected into the Rust actor with their source generation. */
    fun handleTransportEvent(core: HostCoreController, event: FfiHostTransportEvent, sourceHandle: FfiHostTransportHandle) {
        when (event) {
            is FfiHostTransportEvent.JoinRequestReceived -> {
                val requestId = newHostJoinRequestId()
                val tracked = joinRoutes.record(
                    requestId, event.deviceId, event.syncPort, event.audioPort, sourceHandle,
                    controlAddress = event.controlAddress,
                )
                if (!tracked) {
                    // Do not enqueue an unapprovable actor request. Attempt a
                    // generation-bound busy refusal on the original TCP session
                    // instead of silently dropping this accepted join under load.
                    callbacks.commandFailed(
                        "track listener join",
                        IllegalStateException("Pending listener route capacity exceeded"),
                    )
                    refuseUnprojectedJoin(event.deviceId, event.controlAddress, sourceHandle)
                    return
                }
                runCatching {
                    core.submitJoinRequest(
                        FfiJoinRequestInput(
                            requestId = requestId,
                            deviceId = event.deviceId,
                            displayName = event.displayName,
                            inviteCode = event.inviteCode,
                            receivedAtMs = SystemClock.elapsedRealtime().toULong(),
                        ),
                    )
                }.onFailure { error ->
                    joinRoutes.discard(requestId, event.deviceId)
                    callbacks.commandFailed("submit join request", error)
                    // The TCP reader already accepted this join. If the Rust
                    // actor cannot ingest it, leaving the peer pending would
                    // strand its approval spinner indefinitely.
                    refuseUnprojectedJoin(event.deviceId, event.controlAddress, sourceHandle)
                }
            }
            is FfiHostTransportEvent.PeerDisconnectedAt -> event.listenerId?.let { listenerId ->
                // TCP source address identifies the retired peer even when a
                // replacement join claims the same unverified legacy device ID.
                joinRoutes.removePeer(listenerId, event.controlAddress)
                runCatching { core.submitListenerDisconnected(listenerId) }
                    .onFailure { error -> callbacks.commandFailed("submit listener disconnected", error) }
            }
            is FfiHostTransportEvent.PeerDisconnected -> event.listenerId?.let { listenerId ->
                // Address-less synthetic poll failures cannot identify one TCP
                // generation; fail closed rather than reusing stale routes.
                joinRoutes.removeDevice(listenerId)
                runCatching { core.submitListenerDisconnected(listenerId) }
                    .onFailure { error -> callbacks.commandFailed("submit listener disconnected", error) }
            }
            is FfiHostTransportEvent.SyncRequestReceived -> respondToSyncRequest(event, sourceHandle)
            is FfiHostTransportEvent.ResyncNotice -> replayControlsForResync(event, sourceHandle)
            is FfiHostTransportEvent.PeerAccepted,
            is FfiHostTransportEvent.Heartbeat,
            is FfiHostTransportEvent.Rejected,
            -> Unit // best-effort connection diagnostics
        }
    }

    /**
     * A join that cannot enter the authoritative actor must receive a visible
     * native refusal, not remain pending forever. Bind the send to the event's
     * original host generation; a late IO dispatch cannot reject a replacement
     * session. Do not treat a missing handle or zero successful peers as delivery.
     */
    private fun refuseUnprojectedJoin(deviceId: String, controlAddress: String, sourceHandle: FfiHostTransportHandle) {
        scope.launch(ioDispatcher) {
            try {
                val delivery = transport.sendJoinRejectionForGeneration(
                    sourceHandle, deviceId, controlAddress, "host_busy",
                )
                check(delivery != null && delivery.successfulPeers > 0u) {
                    "Host join refusal was not delivered to the original listener"
                }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                callbacks.commandFailed("refuse unavailable join", error)
            }
        }
    }

    /**
     * Keeps the generation captured by the native event queue across IO
     * dispatch: a delayed old-session SyncRequest cannot send into a newly
     * rebound host.
     */
    private fun respondToSyncRequest(event: FfiHostTransportEvent.SyncRequestReceived, sourceHandle: FfiHostTransportHandle) {
        val listenerId = event.listenerId
        if (listenerId == null) {
            callbacks.commandFailed(
                "send sync response",
                IllegalStateException("Clock synchronization request had no authorized listener identity"),
            )
            return
        }
        val t2HostReceiveElapsedMs = SystemClock.elapsedRealtime().toULong()
        val admitted = recoveryDispatch.launchSync(scope, ioDispatcher, sourceHandle, listenerId) {
            try {
                transport.sendSyncResponseToListenerForGeneration(
                    expected = sourceHandle,
                    listenerId = listenerId,
                    correlationId = event.correlationId,
                    t1ListenerSendElapsedMs = event.t1ListenerSendElapsedMs,
                    t2HostReceiveElapsedMs = t2HostReceiveElapsedMs,
                )
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                callbacks.commandFailed("send sync response", error)
            }
        }
        if (!admitted) {
            // Clock probes are retriable; refuse an unbounded backlog of IO
            // continuations and surface a sampled error rather than flooding UI.
            val rejected = recoveryDispatch.rejectedSyncRequests
            if (rejected == 1L || rejected % 64L == 0L) {
                callbacks.commandFailed(
                    "send sync response",
                    IllegalStateException("Host clock-sync delivery saturated ($rejected refusals)"),
                )
            }
        }
    }

    /**
     * An admitted listener can request authoritative control recovery after
     * packet loss or clock resynchronization. Never synthesize a new stream
     * timeline from mutable UI state: the Rust socket cache owns the active
     * StreamStart, pending boundaries and Pause snapshot.
     */
    private fun replayControlsForResync(event: FfiHostTransportEvent.ResyncNotice, sourceHandle: FfiHostTransportHandle) {
        // Duplicate recovery notices for the same listener/session are
        // coalesced: the in-flight native replay reads the authoritative Rust
        // snapshot at send time, so new redundant jobs cannot pile up.
        recoveryDispatch.launchResync(scope, ioDispatcher, sourceHandle, event.listenerId) {
            try {
                // A retired host generation returns null. A stopped host replays
                // its terminal Stop to an admitted listener, never a retired
                // StreamStart; a newly joining listener receives no stale replay.
                transport.replayCachedPlaybackForGeneration(expected = sourceHandle, listenerId = event.listenerId)
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                callbacks.commandFailed("replay listener playback state", error)
            }
        }
    }

    private suspend fun executeTransportEffect(core: HostCoreController, effect: FfiTransportEffect) {
        if (effect is FfiTransportEffect.DeliverJoinApproval) {
            deliverJoinApproval(core, effect)
            return
        }
        val delivery = when (effect) {
            is FfiTransportEffect.DeliverJoinRejection -> {
                val route = joinRoutes.take(effect.requestId, effect.listenerId)
                // A claimed legacy ID can be reused by a newer pending TCP
                // generation. Refuse only the exact socket that originated this
                // actor request; an old decision must never reject the replacement.
                // Routes without a recorded address fail closed, not by ID.
                route?.controlAddress?.let { address ->
                    transport.sendJoinRejectionForGeneration(route.sourceHandle, effect.listenerId, address, effect.reasonCode)
                }
            }
            is FfiTransportEffect.DisconnectListener -> {
                joinRoutes.removeDevice(effect.listenerId)
                transport.disconnectPeer(effect.listenerId, effect.reasonCode)
            }
            is FfiTransportEffect.DeliverJoinApproval -> error("handled above")
        }
        core.transportDeliveryCompleted(
            operationId = when (effect) {
                is FfiTransportEffect.DeliverJoinRejection -> effect.operationId
                is FfiTransportEffect.DisconnectListener -> effect.operationId
                is FfiTransportEffect.DeliverJoinApproval -> error("handled above")
            },
            report = delivery.toFfiDeliveryReport(),
        )
    }

    private suspend fun deliverJoinApproval(core: HostCoreController, effect: FfiTransportEffect.DeliverJoinApproval) {
        if (effect.trustedForFuture) {
            // Legacy v3 has no verified key: report a failed delivery to the actor,
            // not a pending approval or a silently trusted claimed device ID.
            joinRoutes.discard(effect.requestId, effect.listenerId)
            core.transportDeliveryCompleted(effect.operationId, FAILED_DELIVERY)
            callbacks.commandFailed("approve listener", IllegalStateException(LEGACY_V3_DURABLE_TRUST_ERROR))
            return
        }
        // Do not send a successful approval without the UDP ports needed to
        // authorize the SAME peer/session. Approval and route authorization
        // must not straddle a host close/rebind suspend boundary.
        val route = joinRoutes.take(effect.requestId, effect.listenerId)
        val outcome = route?.let {
            transport.approveAndAuthorizeListener(
                effect.listenerId, effect.trustedForFuture, it.syncPort, it.audioPort,
                expectedGeneration = it.sourceHandle,
            )
        }
        val authorized = outcome?.authorized == true
        core.transportDeliveryCompleted(
            effect.operationId,
            if (authorized) outcome?.delivery.toFfiDeliveryReport() else FAILED_DELIVERY,
        )
        if (!authorized) {
            val reason = when {
                route == null -> "Original listener join route missing or retired; approval refused"
                outcome == null -> "Host session replaced before listener approval"
                outcome.failure != null -> outcome.failure
                else -> "Listener approval was not delivered"
            }
            callbacks.commandFailed("authorize listener", IllegalStateException(reason))
            return
        }
        core.submitListenerConnected(
            FfiListenerSummary(
                deviceId = effect.listenerId,
                displayName = callbacks.pendingJoinDisplayName(effect.listenerId) ?: effect.listenerId,
                trustState = if (effect.trustedForFuture) FfiTrustState.TRUSTED else FfiTrustState.SESSION_ONLY,
                transportState = FfiTransportState.CONNECTED,
                synchronization = null,
                lastContactMs = SystemClock.elapsedRealtime().toULong(),
                lastError = null,
            ),
        )
    }

    private suspend fun executeStorageEffect(core: HostCoreController, effect: FfiStorageEffect) {
        when (effect) {
            is FfiStorageEffect.PersistSettings -> runCatching {
                callbacks.saveTuning(effect.settings.toRustStoredSettings())
            }.onSuccess {
                core.settingsSaved(effect.operationId)
            }.onFailure { error ->
                core.storageOperationFailed(effect.operationId, error.message ?: "Failed to persist settings", true)
            }
            // The Android host has no verified, host-scoped listener key store
            // yet, so persisting a claimed device ID would silently elevate a
            // spoofable peer. Fail the effect rather than acknowledge an
            // unverified trust record.
            is FfiStorageEffect.PersistTrustedDevice ->
                core.storageOperationFailed(effect.operationId, LEGACY_V3_DURABLE_TRUST_ERROR, false)
        }
    }

    private companion object {
        val FAILED_DELIVERY = FfiDeliveryReport(intendedPeers = 1u, successfulPeers = 0u, failedPeers = 1u)
    }
}

private fun FfiTuningSettings.toRustStoredSettings() = RustStoredTuningSettings(
    syncSampleWindow = syncSampleWindow.toInt(),
    syncCadenceMs = syncCadenceMs.toLong(),
    startupBufferMs = startupBufferMs.toLong(),
    latePacketThresholdMs = latePacketThresholdMs.toLong(),
    hardResyncThresholdMs = hardResyncThresholdMs.toLong(),
    syncDriftThresholdMs = syncDriftThresholdMs,
    scanWindowMs = scanWindowMs.toLong(),
    updatedAtMs = System.currentTimeMillis(),
)
