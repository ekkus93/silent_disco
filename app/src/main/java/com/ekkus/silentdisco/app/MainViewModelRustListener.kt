package com.ekkus.silentdisco.app

import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.core.audio.AudioFormatSpec
import com.ekkus.silentdisco.core.audio.ListenerPlaybackScheduler
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.protocol.SyncResponsePacket
import com.ekkus.silentdisco.core.rust.ListenerCoreController
import com.ekkus.silentdisco.core.sync.HostTimeMapper
import com.ekkus.silentdisco.core.transport.TransportPorts
import com.ekkus.silentdisco.core.transport.TransportSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiApprovalMode
import com.ekkus.silentdisco.core.uniffi.FfiCoreNotification
import com.ekkus.silentdisco.core.uniffi.FfiCoreSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiListenerLifecycle
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportException
import com.ekkus.silentdisco.core.uniffi.FfiPlatformCompletion
import com.ekkus.silentdisco.core.uniffi.FfiPlatformEffect
import com.ekkus.silentdisco.core.uniffi.FfiSessionAdvertisement
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.launch
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

private const val LOCAL_LISTENER_BIND_ADDRESS = "0.0.0.0"
private const val LISTENER_DISPLAY_NAME = "This Android Listener"

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

private val REQUESTED_OR_LATER = setOf(
    ListenerLifecycleState.JOIN_REQUESTED,
    ListenerLifecycleState.AWAITING_APPROVAL,
    ListenerLifecycleState.APPROVED,
    ListenerLifecycleState.CONNECTING,
)
private val CONNECTED_OR_LATER = setOf(
    ListenerLifecycleState.AWAITING_APPROVAL,
    ListenerLifecycleState.APPROVED,
    ListenerLifecycleState.CONNECTING,
)
private val APPROVED_OR_LATER = setOf(
    ListenerLifecycleState.APPROVED,
    ListenerLifecycleState.CONNECTING,
)

/**
 * Lifecycle states the Rust actor does not yet drive (deferred to Block 23's
 * decoder/scheduler ownership work) -- Kotlin's own playback pipeline is
 * authoritative for these until then.
 */
private val PLAYBACK_TAIL_STATES = setOf(
    ListenerLifecycleState.SYNCING_CLOCK,
    ListenerLifecycleState.BUFFERING,
    ListenerLifecycleState.PLAYING,
    ListenerLifecycleState.RECONNECTING,
    ListenerLifecycleState.DESYNCED,
)

/**
 * Resolves the next `listenerState`, refusing to let a Rust-driven snapshot
 * silently regress the UI out of a Kotlin-owned playback-tail state -- Rust
 * doesn't know about SyncingClock/Buffering/Playing/Reconnecting/Desynced
 * yet, so a repeated snapshot still reporting e.g. Approved would otherwise
 * stomp on genuine playback progress. Disconnected/Error are always accepted
 * since they are real terminal facts regardless of current state.
 */
internal fun nextListenerState(
    current: ListenerLifecycleState,
    incoming: ListenerLifecycleState,
): ListenerLifecycleState {
    val currentIsPlaybackTail = current in PLAYBACK_TAIL_STATES
    val incomingIsRustDriven = incoming !in PLAYBACK_TAIL_STATES &&
        incoming != ListenerLifecycleState.DISCONNECTED &&
        incoming != ListenerLifecycleState.ERROR
    return if (currentIsPlaybackTail && incomingIsRustDriven) current else incoming
}

internal fun MainViewModel.ensureRustListenerCore(): ListenerCoreController {
    listenerCoreController?.let { return it }
    val controller = listenerCoreFactory(localListenerDeviceId)
    listenerCoreController = controller
    viewModelScope.launch {
        controller.snapshots.filterNotNull().collect(::applyRustListenerSnapshot)
    }
    viewModelScope.launch {
        controller.notifications.collect { notification ->
            executeRustListenerNotification(controller, notification)
        }
    }
    return controller
}

private fun MainViewModel.applyRustListenerSnapshot(snapshot: FfiCoreSnapshot) {
    val incoming = snapshot.listenerLifecycle.toAppListenerLifecycle()
    val nextState = nextListenerState(_uiState.value.listenerState, incoming)
    val discoveredSessions = snapshot.discoveredSessions.map { it.toAppSessionInfo() }
    val selectedSession = discoveredSessions.firstOrNull { it.id == snapshot.selectedSession }
    _uiState.value = _uiState.value.copy(
        listenerState = nextState,
        isScanning = snapshot.discoveryActive,
        discoveredSessions = discoveredSessions,
        selectedSession = selectedSession,
        connectionProgress = _uiState.value.connectionProgress.copy(
            currentState = nextState,
            discovered = discoveredSessions.isNotEmpty(),
            requested = nextState in REQUESTED_OR_LATER,
            connected = nextState in CONNECTED_OR_LATER,
            approved = nextState in APPROVED_OR_LATER,
        ),
        lastError = snapshot.lastError?.message,
    )
}

private suspend fun MainViewModel.executeRustListenerNotification(
    controller: ListenerCoreController,
    notification: FfiCoreNotification,
) {
    when (notification) {
        is FfiCoreNotification.Snapshot -> applyRustListenerSnapshot(notification.snapshot)
        is FfiCoreNotification.PlatformEffect -> executeRustListenerPlatformEffect(controller, notification.effect)
        is FfiCoreNotification.TransportEffect -> Unit // the listener role never emits transport effects
        is FfiCoreNotification.StorageEffect -> Unit // the listener role does not persist state yet
        is FfiCoreNotification.Error -> logger.e("rust.listener", notification.error.message)
        is FfiCoreNotification.Diagnostic -> logger.i(
            "rust.listener.${notification.diagnostic.name}",
            notification.diagnostic.fields.joinToString(),
        )
    }
}

private fun MainViewModel.executeRustListenerPlatformEffect(
    controller: ListenerCoreController,
    effect: FfiPlatformEffect,
) {
    when (effect) {
        is FfiPlatformEffect.StartDiscovery -> startRustListenerDiscovery(controller, effect)
        is FfiPlatformEffect.StopDiscovery -> stopRustListenerDiscovery(controller, effect)
        is FfiPlatformEffect.EstablishNetwork -> establishRustListenerNetwork(controller, effect)
        is FfiPlatformEffect.ReleaseNetwork -> releaseRustListenerNetwork(controller, effect)
        is FfiPlatformEffect.StartAdvertising,
        is FfiPlatformEffect.StopAdvertising,
        is FfiPlatformEffect.RequestCapabilities,
        is FfiPlatformEffect.PrepareAudioSource,
        is FfiPlatformEffect.StartAudioOutput,
        is FfiPlatformEffect.StopAudioOutput,
        is FfiPlatformEffect.ShareDiagnostics,
        -> {
            val operationId = when (effect) {
                is FfiPlatformEffect.StartAdvertising -> effect.operationId
                is FfiPlatformEffect.StopAdvertising -> effect.operationId
                is FfiPlatformEffect.RequestCapabilities -> effect.operationId
                is FfiPlatformEffect.PrepareAudioSource -> effect.operationId
                is FfiPlatformEffect.StartAudioOutput -> effect.operationId
                is FfiPlatformEffect.StopAudioOutput -> effect.operationId
                is FfiPlatformEffect.ShareDiagnostics -> effect.operationId
                else -> error("unreachable platform effect")
            }
            controller.platformOperationFailed(
                operationId,
                "Platform effect is outside Android listener Block 13",
                false,
            )
        }
    }
}

private fun MainViewModel.startRustListenerDiscovery(
    controller: ListenerCoreController,
    effect: FfiPlatformEffect.StartDiscovery,
) {
    if (!hasListenerTransportPermissions()) {
        controller.platformOperationFailed(
            effect.operationId,
            "Missing nearby connectivity permissions for discovery",
            true,
        )
        return
    }
    val bleResult = bleService.startScanning()
    if (!bleResult.started) {
        controller.platformOperationFailed(
            effect.operationId,
            bleResult.message ?: "BLE scan could not start",
            true,
        )
        return
    }
    wifiDirectService.discoverPeers()
    controller.platformOperationSucceeded(effect.operationId, FfiPlatformCompletion.DiscoveryStarted)
}

private fun MainViewModel.stopRustListenerDiscovery(
    controller: ListenerCoreController,
    effect: FfiPlatformEffect.StopDiscovery,
) {
    bleService.stopScanning()
    wifiDirectService.cancelDiscovery()
    controller.platformOperationSucceeded(effect.operationId, FfiPlatformCompletion.DiscoveryStopped)
}

private fun MainViewModel.establishRustListenerNetwork(
    controller: ListenerCoreController,
    effect: FfiPlatformEffect.EstablishNetwork,
) {
    val address = effect.address
    val controlPort = effect.controlPort
    val syncPort = effect.syncPort
    val audioPort = effect.audioPort
    if (address != null && controlPort != null && syncPort != null && audioPort != null) {
        // The endpoint was already known (manual/desktop-style connect). The
        // manual-endpoint UI flow still uses its own transport handle for now
        // (see ManualListenerTransportController); this branch just models
        // the actor's contract honestly for when that gets unified.
        controller.platformOperationSucceeded(
            effect.operationId,
            FfiPlatformCompletion.NetworkEndpointReady(address, controlPort, syncPort, audioPort),
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

private fun MainViewModel.releaseRustListenerNetwork(
    controller: ListenerCoreController,
    effect: FfiPlatformEffect.ReleaseNetwork,
) {
    pendingEstablishNetworkOperationId = null
    wifiDirectService.stop()
    controller.platformOperationSucceeded(effect.operationId, FfiPlatformCompletion.NetworkReleased)
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
    val rawEndpoint = Json.encodeToString(
        ManualHostEndpointPayload.serializer(),
        ManualHostEndpointPayload(
            hostAddress = address,
            controlPort = ports.control,
            syncPort = ports.sync,
            audioPort = ports.audio,
            sessionId = session.id,
            protocolVersion = LISTENER_DISCOVERY_PROTOCOL_VERSION,
            inviteCodeRequired = session.inviteCodeRequired,
        ),
    )
    val inviteCode = _uiState.value.connectionProgress.inviteCode.ifBlank { null }
    viewModelScope.launch {
        try {
            listenerTransportController.connect(viewModelScope, rawEndpoint, localListenerDeviceId, LOCAL_LISTENER_BIND_ADDRESS)
            listenerTransportController.sendJoinRequest(LISTENER_DISPLAY_NAME, inviteCode)
        } catch (error: FfiListenerTransportException) {
            controller.platformOperationFailed(operationId, error.message ?: "listener transport connection failed", true)
            return@launch
        }
        ensureListenerTransportEventLoop()
        controller.platformOperationSucceeded(
            operationId,
            FfiPlatformCompletion.NetworkEndpointReady(
                address = address,
                controlPort = ports.control.toUShort(),
                syncPort = ports.sync.toUShort(),
                audioPort = ports.audio.toUShort(),
            ),
        )
    }
}

/**
 * Starts (once) the poll loop that forwards [ListenerTransportController]
 * events into the existing [ListenerCoreController] contract established in
 * Block 13.3 -- the controller itself has zero actor knowledge.
 */
private fun MainViewModel.ensureListenerTransportEventLoop() {
    if (listenerTransportEventLoopStarted) return
    listenerTransportEventLoopStarted = true
    viewModelScope.launch {
        listenerTransportController.events.collect { event ->
            val controller = listenerCoreController ?: return@collect
            handleListenerTransportEvent(controller, event)
        }
    }
}

private fun MainViewModel.handleListenerTransportEvent(
    controller: ListenerCoreController,
    event: FfiListenerTransportEvent,
) {
    when (event) {
        is FfiListenerTransportEvent.Hello -> controller.submitAwaitingApproval()
        is FfiListenerTransportEvent.JoinApproved -> controller.submitJoinApproved(event.trustedForFuture)
        is FfiListenerTransportEvent.JoinRejected -> controller.submitJoinRejected(event.reason)
        is FfiListenerTransportEvent.HostDisconnected -> controller.transportFailed(event.reason, true)
        is FfiListenerTransportEvent.ConnectionClosed -> controller.transportFailed(event.message ?: "Connection closed", true)
        is FfiListenerTransportEvent.Rejected -> controller.transportFailed(event.message, false)
        is FfiListenerTransportEvent.StreamStarted -> handleTransportStreamStarted(event)
        is FfiListenerTransportEvent.Paused -> propagateListenerPlaybackState(
            playbackState = PlaybackState.PAUSED,
            listenerState = _uiState.value.listenerState,
            message = "Host paused the stream",
        )
        is FfiListenerTransportEvent.Stopped -> handleTransportStreamStopped()
        is FfiListenerTransportEvent.SyncResponseReceived -> handleTransportSyncResponse(event)
        is FfiListenerTransportEvent.AudioReceived -> handleTransportAudioReceived(event)
    }
}

private fun MainViewModel.handleTransportStreamStarted(event: FfiListenerTransportEvent.StreamStarted) {
    val session = _uiState.value.selectedSession ?: return
    val streamId = StreamId(event.streamId)
    currentStreamId = streamId
    _uiState.value = _uiState.value.copy(
        listenerState = ListenerLifecycleState.BUFFERING,
        listenerPlaybackState = PlaybackState.BUFFERING,
        connectionProgress = _uiState.value.connectionProgress.copy(
            currentState = ListenerLifecycleState.BUFFERING,
            approved = true,
            connected = true,
            synced = _uiState.value.listenerSyncState.confidence != SyncQualityBadge.UNKNOWN,
            buffered = false,
            playing = false,
        ),
        lastMessage = "Host stream starting",
        lastError = null,
    )
    startListenerPlaybackFromTransport(
        sessionId = SessionId(session.id),
        streamId = streamId,
        format = AudioFormatSpec(
            sampleRate = event.sampleRate.toInt(),
            channelCount = event.channels.toInt(),
        ),
    )
}

private fun MainViewModel.handleTransportStreamStopped() {
    playbackJob?.cancel()
    listenerScheduler = null
    pendingTransportPackets.clear()
    propagateListenerPlaybackState(
        playbackState = PlaybackState.STOPPED,
        listenerState = ListenerLifecycleState.CONNECTING,
        message = "Host stopped the stream",
    )
    diagnosticsStore.updateListener {
        it.copy(endOfStreamReached = true, playbackState = PlaybackState.STOPPED)
    }
    refreshListenerDiagnostics()
}

private fun MainViewModel.handleTransportSyncResponse(event: FfiListenerTransportEvent.SyncResponseReceived) {
    val session = _uiState.value.selectedSession ?: return
    applySyncResponse(
        SyncResponsePacket(
            version = LISTENER_DISCOVERY_PROTOCOL_VERSION,
            sessionId = SessionId(session.id),
            correlationId = event.correlationId.toLong(),
            t1ListenerSendElapsedMs = event.t1ListenerSendElapsedMs.toLong(),
            t2HostReceiveElapsedMs = event.t2HostReceiveElapsedMs.toLong(),
            t3HostSendElapsedMs = event.t3HostSendElapsedMs.toLong(),
        ),
    )
}

private fun MainViewModel.handleTransportAudioReceived(event: FfiListenerTransportEvent.AudioReceived) {
    val session = _uiState.value.selectedSession ?: return
    val packet = AudioPacket(
        version = LISTENER_DISCOVERY_PROTOCOL_VERSION,
        sessionId = SessionId(session.id),
        streamId = StreamId(event.streamId),
        sequenceNumber = event.sequence.toLong(),
        codec = "pcm16le",
        sampleRate = event.sampleRate.toInt(),
        channelCount = event.channels.toInt(),
        samplesPerPacket = event.samplesPerPacket.toInt(),
        firstSampleIndex = event.firstSampleIndex.toLong(),
        hostPresentationTimeMs = event.hostPresentationTimeMs.toLong(),
        payload = event.payload,
    )
    val scheduler = listenerScheduler
    if (scheduler == null) {
        pendingTransportPackets += packet
        while (pendingTransportPackets.size > 256) {
            pendingTransportPackets.removeFirst()
        }
        return
    }
    recordIncomingPacket(scheduler, packet)
}

/**
 * Starts local playback of the Rust-transport-delivered stream just
 * announced by [FfiListenerTransportEvent.StreamStarted]. Revives the
 * pre-Block-20 `startTransportListenerPlayback`, now driven by the Rust
 * listener transport's own events instead of the deleted Kotlin socket path.
 */
private fun MainViewModel.startListenerPlaybackFromTransport(
    sessionId: SessionId,
    streamId: StreamId,
    format: AudioFormatSpec,
) {
    val mapper = HostTimeMapper(
        offsetMs = _uiState.value.listenerSyncState.offsetMs,
        skewPpm = _uiState.value.listenerSyncState.skewPpm,
    )
    val scheduler = ListenerPlaybackScheduler(
        mapper = mapper,
        thresholds = currentPlaybackThresholds(),
        expectedSessionId = sessionId,
        expectedStreamId = streamId,
    )
    listenerScheduler = scheduler
    pendingTransportPackets
        .filter { it.sessionId == sessionId && it.streamId == streamId }
        .forEach { packet -> recordIncomingPacket(scheduler, packet) }
    pendingTransportPackets.clear()
    runCatching { playbackEngine.start(format) }.onFailure { error ->
        handleListenerPlaybackEngineFailure(error)
        return
    }
    playbackJob?.cancel()
    playbackJob = viewModelScope.launch {
        var started = false
        var lastUnderrunCount = 0
        while (_uiState.value.listenerState != ListenerLifecycleState.DISCONNECTED) {
            if (wifiDirectService.snapshot.value.state == TransportConnectionState.DISCONNECTED ||
                wifiDirectService.snapshot.value.state == TransportConnectionState.FAILED
            ) {
                handleListenerDisconnect("Transport disconnected during playback")
                return@launch
            }
            val activeScheduler = listenerScheduler ?: return@launch
            if (!started) {
                if (!activeScheduler.canStart()) {
                    delay(10)
                    continue
                }
                started = true
                _uiState.value = _uiState.value.copy(
                    listenerState = ListenerLifecycleState.PLAYING,
                    listenerPlaybackState = PlaybackState.PLAYING,
                    connectionProgress = _uiState.value.connectionProgress.copy(
                        currentState = ListenerLifecycleState.PLAYING,
                        connected = true,
                        approved = true,
                        synced = true,
                        buffered = true,
                        playing = true,
                    ),
                )
            }
            val frame = activeScheduler.poll()
            if (frame == null) {
                delay(10)
                continue
            }
            runCatching { playbackEngine.write(frame) }.onFailure { error ->
                handleListenerPlaybackEngineFailure(error)
                return@launch
            }
            val telemetry = activeScheduler.snapshot()
            if (frame.concealed) {
                logger.w("packet.receive.anomaly", "Inserted concealment for seq=${frame.packet.sequenceNumber}")
            }
            if (telemetry.underrunCount > lastUnderrunCount) {
                logger.w("playback.underrun", "Underrun count=${telemetry.underrunCount}")
                lastUnderrunCount = telemetry.underrunCount
            }
            diagnosticsStore.updateListener {
                it.copy(
                    playbackState = if (telemetry.underrunCount > 0) PlaybackState.UNDERRUN else PlaybackState.PLAYING,
                    playbackPositionMs = playbackEngine.playbackPositionMs(frame),
                    bufferDepthMs = telemetry.bufferDepthMs,
                    packetLossCount = telemetry.packetLossCount,
                    lateDropCount = telemetry.lateDropCount,
                    underrunCount = telemetry.underrunCount,
                    invalidPacketCount = telemetry.invalidPacketCount,
                    concealedPacketCount = telemetry.concealedPacketCount,
                    lastPacketSequence = telemetry.lastPlayedSequence,
                    metricsSummary = summarizeMetrics(),
                )
            }
            if (telemetry.shouldResync) {
                _uiState.value = _uiState.value.copy(listenerState = ListenerLifecycleState.DESYNCED)
            }
            refreshListenerDiagnostics()
            delay(20)
        }
    }
}

internal fun FfiListenerLifecycle.toAppListenerLifecycle(): ListenerLifecycleState = when (this) {
    FfiListenerLifecycle.IDLE -> ListenerLifecycleState.IDLE
    FfiListenerLifecycle.SCANNING -> ListenerLifecycleState.SCANNING
    FfiListenerLifecycle.SESSION_SELECTED -> ListenerLifecycleState.SESSION_SELECTED
    FfiListenerLifecycle.JOIN_REQUESTED -> ListenerLifecycleState.JOIN_REQUESTED
    FfiListenerLifecycle.AWAITING_APPROVAL -> ListenerLifecycleState.AWAITING_APPROVAL
    FfiListenerLifecycle.APPROVED -> ListenerLifecycleState.APPROVED
    FfiListenerLifecycle.CONNECTING -> ListenerLifecycleState.CONNECTING
    FfiListenerLifecycle.SYNCING_CLOCK -> ListenerLifecycleState.SYNCING_CLOCK
    FfiListenerLifecycle.BUFFERING -> ListenerLifecycleState.BUFFERING
    FfiListenerLifecycle.PLAYING -> ListenerLifecycleState.PLAYING
    FfiListenerLifecycle.RECONNECTING -> ListenerLifecycleState.RECONNECTING
    FfiListenerLifecycle.DESYNCED -> ListenerLifecycleState.DESYNCED
    FfiListenerLifecycle.DISCONNECTED -> ListenerLifecycleState.DISCONNECTED
    FfiListenerLifecycle.ERROR -> ListenerLifecycleState.ERROR
}

internal fun FfiSessionAdvertisement.toAppSessionInfo(): SessionInfo = SessionInfo(
    id = sessionId,
    name = sessionName,
    hostDeviceName = hostDeviceId,
    approvalMode = approvalMode.toAppApprovalMode(),
    inviteCodeRequired = approvalMode == FfiApprovalMode.INVITE_CODE,
)

/** Protocol version advertised by this app's own BLE/Wi-Fi Direct codec. */
private const val LISTENER_DISCOVERY_PROTOCOL_VERSION: Int = 2

internal fun SessionInfo.toFfiSessionAdvertisement(): FfiSessionAdvertisement = FfiSessionAdvertisement(
    sessionId = id,
    hostDeviceId = hostDeviceName,
    sessionName = name,
    approvalMode = approvalMode.toFfiApprovalMode(),
    protocolVersion = LISTENER_DISCOVERY_PROTOCOL_VERSION.toUShort(),
    address = null,
    controlPort = null,
    syncPort = null,
    audioPort = null,
)
