package com.ekkus.silentdisco.app

import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SyncState
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.protocol.SyncResponsePacket
import com.ekkus.silentdisco.core.rust.ListenerCoreController
import com.ekkus.silentdisco.core.transport.MdnsSessionAdvertisement
import com.ekkus.silentdisco.core.uniffi.FfiApprovalMode
import com.ekkus.silentdisco.core.uniffi.FfiCoreNotification
import com.ekkus.silentdisco.core.uniffi.FfiCoreSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiListenerLifecycle
import com.ekkus.silentdisco.core.uniffi.FfiPlaybackDiagnostics
import com.ekkus.silentdisco.core.uniffi.FfiPlaybackPhase
import com.ekkus.silentdisco.core.uniffi.FfiSyncSampleOutcome
import com.ekkus.silentdisco.core.uniffi.FfiSyncConfidence
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiPlatformEffect
import com.ekkus.silentdisco.core.uniffi.FfiSessionAdvertisement
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.launch

private const val CANONICAL_STREAM_SAMPLE_RATE = 48_000L

private val REQUESTED_OR_LATER = setOf(
    ListenerLifecycleState.JOIN_REQUESTED,
    ListenerLifecycleState.AWAITING_APPROVAL,
    ListenerLifecycleState.APPROVED,
    ListenerLifecycleState.CONNECTING,
)
// The core's join order is JOIN_REQUESTED -> CONNECTING (endpoint ready) ->
// AWAITING_APPROVAL -> APPROVED. CONNECTING precedes approval, so it must not
// count as approved: doing so ticked "Waiting for host approval" before the
// host had decided anything (Block 52.5).
private val CONNECTED_OR_LATER = setOf(
    ListenerLifecycleState.CONNECTING,
    ListenerLifecycleState.AWAITING_APPROVAL,
    ListenerLifecycleState.APPROVED,
)
private val APPROVED_OR_LATER = setOf(
    ListenerLifecycleState.APPROVED,
)

internal fun listenerBoundaryLocalTimeMs(
    hostStartTimeMs: Long,
    firstSampleIndex: Long,
    hostOffsetMs: Double,
): Double =
    hostStartTimeMs + firstSampleIndex * 1_000.0 / CANONICAL_STREAM_SAMPLE_RATE - hostOffsetMs

internal fun shouldReanchorListenerPlayback(
    resumeInPlace: Boolean,
    runtimeAvailable: Boolean,
): Boolean = resumeInPlace && runtimeAvailable

internal fun listenerTrackNameAfterStreamStart(
    isSameStream: Boolean,
    currentTrackName: String?,
    announcedTrackName: String?,
): String? {
    val announced = announcedTrackName?.takeIf(String::isNotBlank)
    return when {
        announced != null -> announced
        isSameStream -> currentTrackName
        else -> null
    }
}

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

internal fun MainViewModel.applyRustListenerSnapshot(snapshot: FfiCoreSnapshot) {
    val discoveryJustStarted = snapshot.discoveryActive && !_uiState.value.isScanning
    val wasApproved = _uiState.value.listenerState in APPROVED_OR_LATER
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
    if (!wasApproved && nextState in APPROVED_OR_LATER && listenerTransportConnected) {
        // Start acquiring the host clock as soon as admission completes. This
        // lets a listener lock sync while the host waits before pressing Play.
        startPeriodicListenerResync()
    }
    // Reconciliation is gated on the core discovering, so hand it anything
    // the platform scanners already found at the moment discovery starts.
    if (discoveryJustStarted) {
        refreshDiscoveredSessions()
    }
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

/**
 * Starts (once) the poll loop that forwards [ListenerTransportController]
 * events into the existing [ListenerCoreController] contract established in
 * Block 13.3 -- the controller itself has zero actor knowledge.
 */
internal fun MainViewModel.ensureListenerTransportEventLoop() {
    if (listenerTransportEventLoopStarted) return
    listenerTransportEventLoopStarted = true
    viewModelScope.launch(Dispatchers.IO) {
        listenerTransportController.events.collect { event ->
            val controller = listenerCoreController ?: return@collect
            handleListenerTransportEvent(controller, event)
        }
    }
}

internal fun MainViewModel.handleListenerTransportEvent(
    controller: ListenerCoreController,
    event: FfiListenerTransportEvent,
) {
    when (event) {
        is FfiListenerTransportEvent.Hello -> controller.submitAwaitingApproval()
        is FfiListenerTransportEvent.JoinApproved -> controller.submitJoinApproved(event.trustedForFuture)
        is FfiListenerTransportEvent.JoinRejected -> controller.submitJoinRejected(event.reason)
        is FfiListenerTransportEvent.HostDisconnected -> leaveSession()
        is FfiListenerTransportEvent.ConnectionClosed -> {
            val message = event.message
            if (message == null) {
                // A clean peer close is the normal result when a host ends its
                // session. Release the listener network and return to the
                // nearby-session flow instead of presenting a recoverable
                // playback error for an intentional shutdown.
                leaveSession()
            } else {
                controller.transportFailed(message, true)
            }
        }
        is FfiListenerTransportEvent.Rejected -> controller.transportFailed(event.message, false)
        is FfiListenerTransportEvent.StreamStarted -> handleTransportStreamStarted(event)
        is FfiListenerTransportEvent.TrackBoundary -> handleTransportTrackBoundary(event)
        is FfiListenerTransportEvent.Paused -> {
            cancelListenerTrackBoundaryTimers()
            propagateListenerPlaybackState(
                playbackState = PlaybackState.PAUSED,
                listenerState = _uiState.value.listenerState,
                message = "Host paused the stream",
            )
        }
        is FfiListenerTransportEvent.Stopped -> handleTransportStreamStopped()
        is FfiListenerTransportEvent.SyncResponseReceived -> handleTransportSyncResponse(event)
        is FfiListenerTransportEvent.AudioReceived -> handleTransportAudioReceived(event)
    }
}

private fun MainViewModel.handleTransportStreamStarted(event: FfiListenerTransportEvent.StreamStarted) {
    val session = _uiState.value.selectedSession ?: return
    val streamId = StreamId(event.streamId)
    val isSameStream = currentStreamId == streamId
    currentStreamId = streamId
    listenerStreamHostStartTimeMs = event.hostStartTimeMs.toLong()
    _uiState.value = _uiState.value.copy(
        listenerCurrentTrackName = listenerTrackNameAfterStreamStart(
            isSameStream = isSameStream,
            currentTrackName = _uiState.value.listenerCurrentTrackName,
            announcedTrackName = event.currentDisplayName,
        ),
    )
    if (!isSameStream) {
        pendingListenerTrackBoundaries.clear()
    }
    rescheduleListenerTrackBoundaries()
    if (!isSameStream || listenerPlayback == null) {
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
    }
    startListenerPlaybackFromTransport(
        sessionId = SessionId(session.id),
        streamId = streamId,
        event = event,
        resumeInPlace = isSameStream,
    )
}

internal fun MainViewModel.handleTransportStreamStopped() {
    // The runtime drains its own buffered tail while stopping, so the
    // stream's final moments play rather than being discarded.
    // A runtime's clock has a per-runtime epoch. Keep its periodic loop
    // from switching to the pre-runtime estimator after Stop: comparing
    // that estimator's Android elapsed-realtime offset with the runtime's
    // offset falsely marks an otherwise healthy listener DESYNCED. The next
    // StreamStart creates a new runtime and starts its own probe loop.
    resyncJob?.cancel()
    resyncJob = null
    listenerStreamHostStartTimeMs = null
    pendingListenerTrackBoundaries.clear()
    cancelListenerTrackBoundaryTimers()
    stopListenerPlayback()
    propagateListenerPlaybackState(
        playbackState = PlaybackState.STOPPED,
        listenerState = ListenerLifecycleState.APPROVED,
        message = "Host stopped the stream",
    )
    _uiState.value = _uiState.value.copy(
        connectionProgress = _uiState.value.connectionProgress.copy(
            currentState = ListenerLifecycleState.APPROVED,
            synced = _uiState.value.listenerSyncState.confidence != SyncQualityBadge.UNKNOWN,
        ),
    )
    diagnosticsStore.updateListener {
        it.copy(endOfStreamReached = true, playbackState = PlaybackState.STOPPED)
    }
    refreshListenerDiagnostics()
}

private fun MainViewModel.handleTransportTrackBoundary(
    event: FfiListenerTransportEvent.TrackBoundary,
) {
    if (currentStreamId?.value != event.streamId) return
    val sampleIndex = event.firstSampleIndex.toLong()
    metrics.increment("track_boundary_received")
    pendingListenerTrackBoundaries[sampleIndex] = PendingListenerTrackBoundary(
        streamId = event.streamId,
        firstSampleIndex = sampleIndex,
        displayName = event.displayName,
    )
    scheduleListenerTrackBoundary(sampleIndex)
}

internal fun MainViewModel.rescheduleListenerTrackBoundaries() {
    cancelListenerTrackBoundaryTimers()
    pendingListenerTrackBoundaries.keys.toList().forEach(::scheduleListenerTrackBoundary)
}

private fun MainViewModel.cancelListenerTrackBoundaryTimers() {
    listenerTrackBoundaryJobs.values.forEach { it.cancel() }
    listenerTrackBoundaryJobs.clear()
}

private fun MainViewModel.scheduleListenerTrackBoundary(sampleIndex: Long) {
    if (_uiState.value.listenerPlaybackState == PlaybackState.PAUSED) return
    if (_uiState.value.listenerSyncState.confidence == SyncQualityBadge.UNKNOWN) return
    val boundary = pendingListenerTrackBoundaries[sampleIndex] ?: return
    val hostStartMs = listenerStreamHostStartTimeMs ?: return
    val boundaryLocalTimeMs = listenerBoundaryLocalTimeMs(
        hostStartMs,
        sampleIndex,
        _uiState.value.listenerSyncState.offsetMs,
    )
    val waitMs = (boundaryLocalTimeMs - android.os.SystemClock.elapsedRealtime())
        .coerceAtLeast(0.0)
        .toLong()
    listenerTrackBoundaryJobs.remove(sampleIndex)?.cancel()
    metrics.increment("track_boundary_scheduled")
    listenerTrackBoundaryJobs[sampleIndex] = viewModelScope.launch {
        delay(waitMs)
        if (currentStreamId?.value != boundary.streamId) return@launch
        if (_uiState.value.listenerPlaybackState == PlaybackState.PAUSED) return@launch
        pendingListenerTrackBoundaries.remove(sampleIndex)
        listenerTrackBoundaryJobs.remove(sampleIndex)
        _uiState.value = _uiState.value.copy(listenerCurrentTrackName = boundary.displayName)
        metrics.increment("track_boundary_applied")
        diagnosticsStore.updateListener { it.copy(metricsSummary = summarizeMetrics()) }
        refreshListenerDiagnostics()
    }
}

/**
 * Feeds one four-timestamp exchange to whichever component owns the estimate.
 *
 * Once a playback runtime exists it is the only authority: it holds the
 * estimator, and playback stays gated until it accepts a sample. The Kotlin
 * controller below is used only before a stream starts, where there is no
 * runtime yet but the connection-progress UI still needs to show that sync is
 * under way. Nothing derives playback timing from it.
 */
private fun MainViewModel.handleTransportSyncResponse(event: FfiListenerTransportEvent.SyncResponseReceived) {
    val session = _uiState.value.selectedSession ?: return
    val runtime = listenerPlayback
    if (runtime == null) {
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
        val sync = _uiState.value.listenerSyncState
        if (sync.confidence != SyncQualityBadge.UNKNOWN) {
            reportListenerSyncToHost(
                sync.confidence.toFfiSyncConfidence(),
                sync.offsetMs,
                sync.rttMs,
                sync.skewPpm,
            )
        }
        return
    }
    val outcome = runCatching {
        runtime.observeSyncResponse(
            event.correlationId,
            event.t1ListenerSendElapsedMs,
            event.t2HostReceiveElapsedMs,
            event.t3HostSendElapsedMs,
            runtime.nowMs(),
        )
    }.getOrElse { error ->
        logger.w("sync.rejected", error.message ?: "sync response rejected")
        return
    }
    reportListenerSyncToHost(
        outcome.confidence,
        outcome.offsetMs,
        outcome.roundTripTimeMs,
        outcome.skewPpm,
    )
    applyRuntimeSyncOutcome(outcome)
}

internal fun MainViewModel.reportListenerSyncToHost(
    confidence: FfiSyncConfidence,
    offsetMs: Double,
    roundTripMs: Double,
    driftPpm: Double,
) {
    try {
        listenerTransportController.sendSynchronizationReport(
            confidence,
            offsetMs,
            roundTripMs,
            driftPpm,
        )
    } catch (error: RuntimeException) {
        logger.w("sync.report_send_failed", error.message ?: "synchronization report send failed")
    }
}

internal fun SyncQualityBadge.toFfiSyncConfidence(): FfiSyncConfidence = when (this) {
    SyncQualityBadge.UNKNOWN -> FfiSyncConfidence.UNKNOWN
    SyncQualityBadge.POOR -> FfiSyncConfidence.POOR
    SyncQualityBadge.FAIR -> FfiSyncConfidence.FAIR
    SyncQualityBadge.GOOD -> FfiSyncConfidence.GOOD
    SyncQualityBadge.EXCELLENT -> FfiSyncConfidence.EXCELLENT
}

/** Mirrors the runtime's own estimate into the UI and diagnostics. */
private fun MainViewModel.applyRuntimeSyncOutcome(outcome: FfiSyncSampleOutcome) {
    logger.i(
        "sync.sample",
        "accepted=${outcome.accepted} offset=${"%.2f".format(outcome.offsetMs)} " +
            "rtt=${"%.2f".format(outcome.roundTripTimeMs)} skewPpm=${"%.2f".format(outcome.skewPpm)} " +
            "confidence=${outcome.confidence} locked=${outcome.syncLocked} " +
            "acquisitionRejected=${outcome.acquisitionRejectedSampleCount} " +
            "acquisitionElapsedMs=${outcome.acquisitionElapsedMs} " +
            "acquisitionRttLimitMs=${"%.0f".format(outcome.acquisitionRttLimitMs)} " +
            "degradedLock=${outcome.degradedLock}",
    )
    if (!outcome.accepted) {
        val message =
            "Checking connection timing…"
        _uiState.value = _uiState.value.copy(
            listenerState = ListenerLifecycleState.SYNCING_CLOCK,
            connectionProgress = _uiState.value.connectionProgress.copy(
                currentState = ListenerLifecycleState.SYNCING_CLOCK,
                synced = false,
                playing = false,
            ),
            lastMessage = message,
            lastError = null,
        )
        return
    }
    val syncState = SyncState(
        offsetMs = outcome.offsetMs,
        rttMs = outcome.roundTripTimeMs,
        jitterMs = outcome.jitterMs,
        skewPpm = outcome.skewPpm,
        confidence = outcome.confidence.toAppSyncQuality(),
        resyncCount = _uiState.value.listenerSyncState.resyncCount + 1,
    )
    _uiState.value = _uiState.value.copy(
        listenerSyncState = syncState,
        connectionProgress = _uiState.value.connectionProgress.copy(synced = outcome.syncLocked),
        lastMessage = if (outcome.degradedLock) {
            "Connected with limited timing accuracy"
        } else {
            "Connection timing is ready"
        },
        lastError = null,
    )
    rescheduleListenerTrackBoundaries()
    diagnosticsStore.updateListener {
        it.copy(
            hostOffsetMs = syncState.offsetMs,
            rttMs = syncState.rttMs,
            jitterMs = syncState.jitterMs,
            resyncCount = syncState.resyncCount,
            metricsSummary = summarizeMetrics(),
        )
    }
    metrics.increment("sync_sample")
    metrics.recordTiming("sync_rtt_ms", syncState.rttMs)
    refreshListenerDiagnostics()
}

/**
 * Maps the runtime's own state to a reported playback state.
 *
 * Derived rather than assumed: buffering and awaiting-sync are real states a
 * listener can be stuck in, and collapsing them into PLAYING is what let a
 * permanently silent stream report itself healthy.
 */
internal fun FfiPlaybackDiagnostics.toPlaybackState(): PlaybackState = when {
    !syncLocked -> PlaybackState.BUFFERING
    phase == FfiPlaybackPhase.STOPPED -> PlaybackState.STOPPED
    phase == FfiPlaybackPhase.BUFFERING || phase == FfiPlaybackPhase.AWAITING_REBUFFER ->
        PlaybackState.BUFFERING
    else -> PlaybackState.PLAYING
}

internal fun FfiSyncConfidence.toAppSyncQuality(): SyncQualityBadge = when (this) {
    FfiSyncConfidence.UNKNOWN -> SyncQualityBadge.UNKNOWN
    FfiSyncConfidence.POOR -> SyncQualityBadge.POOR
    FfiSyncConfidence.FAIR -> SyncQualityBadge.FAIR
    FfiSyncConfidence.GOOD -> SyncQualityBadge.GOOD
    FfiSyncConfidence.EXCELLENT -> SyncQualityBadge.EXCELLENT
}

/**
 * Render ring geometry and pacing for the discovered-session listener,
 * matching the manual-connect path: one second of capacity with a 400ms
 * cushion, handed to the Rust runtime that owns every decision made with it.
 */
/** Span rebuilt before a *mid-stream* rebuffer resumes; see the manual controller's own constant. */
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

internal fun MdnsSessionAdvertisement.toFfiSessionAdvertisement(): FfiSessionAdvertisement =
    FfiSessionAdvertisement(
        sessionId = sessionId,
        hostDeviceId = hostDeviceId,
        sessionName = sessionName,
        approvalMode = approvalMode.toFfiApprovalMode(),
        protocolVersion = protocolVersion.toUShort(),
        address = address,
        controlPort = controlPort.toUShort(),
        syncPort = syncPort.toUShort(),
        audioPort = audioPort.toUShort(),
    )
