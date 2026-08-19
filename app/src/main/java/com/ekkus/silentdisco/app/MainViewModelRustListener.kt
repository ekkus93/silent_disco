package com.ekkus.silentdisco.app

import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.audio.OboeBridge
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SyncState
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.protocol.SyncResponsePacket
import com.ekkus.silentdisco.core.rust.ListenerCoreController
import com.ekkus.silentdisco.core.transport.MdnsSessionAdvertisement
import com.ekkus.silentdisco.core.uniffi.FfiApprovalMode
import com.ekkus.silentdisco.core.uniffi.FfiCoreNotification
import com.ekkus.silentdisco.core.uniffi.FfiCoreSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiListenerLifecycle
import com.ekkus.silentdisco.core.uniffi.FfiAudioPacket
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackConfig
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
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
        event = event,
    )
}

private fun MainViewModel.handleTransportStreamStopped() {
    // The runtime drains its own buffered tail while stopping, so the
    // stream's final moments play rather than being discarded.
    stopListenerPlayback()
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
    applyRuntimeSyncOutcome(outcome)
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
            "Clock sync: ${outcome.acquisitionRejectedSampleCount} rejected over " +
                "${outcome.acquisitionElapsedMs}ms (RTT gate " +
                "${"%.0f".format(outcome.acquisitionRttLimitMs)}ms)"
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
            "Clock sync locked after ${outcome.acquisitionRejectedSampleCount} rejected samples " +
                "using a bounded degraded acquisition gate"
        } else {
            "Clock sync locked"
        },
        lastError = null,
    )
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
private const val LISTENER_REBUFFER_TARGET_MS: ULong = 400uL

private const val LISTENER_RING_CAPACITY_FRAMES: UInt = 48_000u
private const val LISTENER_RING_TARGET_FILL_FRAMES: UInt = 19_200u
private const val LISTENER_RING_WRITE_LEAD_MS: ULong = 400uL
private const val LISTENER_MAX_RING_PREFILL_MS: ULong = 800uL

/** How long a stream may sit without a clock lock before it is reported. */
private const val LISTENER_SYNC_LOCK_TIMEOUT_MS = 5_000L

/** How often the runtime's own accounting is mirrored into the UI. */
private const val LISTENER_DIAGNOSTICS_POLL_MS = 100L

/** `nativeOboeOpen` success status. */
private const val LISTENER_OBOE_STATUS_OK = 0

private fun MainViewModel.handleTransportAudioReceived(event: FfiListenerTransportEvent.AudioReceived) {
    val session = _uiState.value.selectedSession ?: return
    val runtime = listenerPlayback ?: return
    // Ordering, concealment, and scheduling belong to the runtime; a packet it
    // rejects is ordinary traffic its own counters already account for.
    runCatching {
        runtime.submitPacket(
            FfiAudioPacket(
                sequence = event.sequence,
                sampleRate = event.sampleRate,
                channels = event.channels,
                samplesPerPacket = event.samplesPerPacket,
                firstSampleIndex = event.firstSampleIndex,
                hostPresentationTimeMs = event.hostPresentationTimeMs,
                payload = event.payload,
            ),
            session.id,
            event.streamId,
        )
    }.onFailure { error -> handleListenerPlaybackEngineFailure(error) }
}

/**
 * Starts Rust-owned playback of the stream just announced by
 * [FfiListenerTransportEvent.StreamStarted].
 *
 * Everything between an arriving packet and the render ring — ordering,
 * concealment, presentation-time pacing, clock estimation, PCM conversion —
 * belongs to the runtime. This function only opens it, points the native
 * output at its ring, and reflects its state into the UI.
 */
internal fun MainViewModel.startListenerPlaybackFromTransport(
    sessionId: SessionId,
    streamId: StreamId,
    event: FfiListenerTransportEvent.StreamStarted,
) {
    listenerPlaybackControlExecutor.execute {
        startListenerPlaybackFromTransportNow(sessionId, streamId, event)
    }
}

private fun MainViewModel.startListenerPlaybackFromTransportNow(
    sessionId: SessionId,
    streamId: StreamId,
    event: FfiListenerTransportEvent.StreamStarted,
) {
    stopListenerPlaybackNow()
    val runtime = runCatching {
        FfiListenerPlaybackHandle.open(
            FfiListenerPlaybackConfig(
                sessionId = sessionId.value,
                streamId = streamId.value,
                sampleRate = event.sampleRate,
                hostStartTimeMs = event.hostStartTimeMs,
                samplesPerPacket = event.samplesPerPacket,
                channels = event.channels,
                startupBufferTargetMs = currentPlaybackThresholds().startupBufferMs.toULong(),
                // Clamped by the scheduler to the startup target above, so a tuning
                // profile with a shallower startup buffer keeps its fast recovery.
                rebufferTargetMs = LISTENER_REBUFFER_TARGET_MS,
                ringCapacityFrames = LISTENER_RING_CAPACITY_FRAMES,
                ringTargetFillFrames = LISTENER_RING_TARGET_FILL_FRAMES,
                writeLeadMs = LISTENER_RING_WRITE_LEAD_MS,
                maxPrefillMs = LISTENER_MAX_RING_PREFILL_MS,
                volume = 1.0f,
            ),
        )
    }.getOrElse { error ->
        handleListenerPlaybackEngineFailure(error)
        return
    }
    listenerPlayback = runtime

    val oboeStatus = OboeBridge.nativeOboeOpen(runtime.engineToken())
    if (oboeStatus != LISTENER_OBOE_STATUS_OK) {
        stopListenerPlaybackNow()
        handleListenerPlaybackEngineFailure(
            IllegalStateException("Oboe stream failed to open (status=$oboeStatus)"),
        )
        return
    }
    try {
        listenerTransportController.attachPlayback(runtime)
    } catch (error: Throwable) {
        stopListenerPlaybackNow()
        handleListenerPlaybackEngineFailure(error)
        return
    }
    _uiState.value = _uiState.value.copy(
        listenerState = ListenerLifecycleState.BUFFERING,
        listenerPlaybackState = PlaybackState.BUFFERING,
    )
    startListenerPlaybackDiagnostics(runtime)
}

/**
 * Mirrors the runtime's own accounting into the diagnostics store and UI.
 *
 * Polling a snapshot beats deriving state per frame: the counters come from
 * the component that owns the behaviour they describe, so they cannot drift
 * from what actually happened.
 */
private fun MainViewModel.startListenerPlaybackDiagnostics(runtime: FfiListenerPlaybackHandle) {
    playbackJob?.cancel()
    playbackJob = viewModelScope.launch(Dispatchers.IO) {
        var reportedPlaying = false
        var reportedSyncStall = false
        var lastUnderruns = 0UL
        var lastDroppedBeforeSync = 0UL
        val openedAtMs = android.os.SystemClock.elapsedRealtime()
        while (_uiState.value.listenerState != ListenerLifecycleState.DISCONNECTED) {
            if (wifiDirectService.snapshot.value.state == TransportConnectionState.DISCONNECTED ||
                wifiDirectService.snapshot.value.state == TransportConnectionState.FAILED
            ) {
                handleListenerDisconnect("Transport disconnected during playback")
                return@launch
            }
            if (listenerPlayback !== runtime) return@launch
            val diagnostics = runtime.diagnostics()
            // A stream that cannot play must never read as playing. Without a
            // real clock offset the pump releases nothing at all, and
            // reporting PLAYING there turns a hard failure into an invisible
            // one -- which is how this path shipped silent.
            if (!diagnostics.syncLocked &&
                android.os.SystemClock.elapsedRealtime() - openedAtMs > LISTENER_SYNC_LOCK_TIMEOUT_MS &&
                !reportedSyncStall
            ) {
                reportedSyncStall = true
                val message = "No clock sync established; playback cannot start"
                logger.w("sync.stalled", message)
                _uiState.value = _uiState.value.copy(
                    listenerState = ListenerLifecycleState.DESYNCED,
                    lastError = message,
                )
                diagnosticsStore.updateListener { it.copy(lastError = message) }
            }
            if (!diagnostics.syncLocked && diagnostics.droppedBeforeSync > lastDroppedBeforeSync) {
                val message =
                    "Waiting for clock sync; ${diagnostics.droppedBeforeSync} audio packets were dropped before lock"
                logger.w("sync.pre_sync_drop", message)
                _uiState.value = _uiState.value.copy(
                    listenerState = ListenerLifecycleState.SYNCING_CLOCK,
                    lastMessage = message,
                )
            }
            lastDroppedBeforeSync = diagnostics.droppedBeforeSync
            if (!reportedPlaying && diagnostics.phase == FfiPlaybackPhase.PLAYING) {
                reportedPlaying = true
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
            if (diagnostics.ringUnderruns > lastUnderruns) {
                logger.w("playback.underrun", "Render ring underruns=${diagnostics.ringUnderruns}")
                lastUnderruns = diagnostics.ringUnderruns
            }
            diagnosticsStore.updateListener {
                it.copy(
                    playbackState = diagnostics.toPlaybackState(),
                    bufferDepthMs = diagnostics.bufferedSpanMs.toLong(),
                    packetLossCount = diagnostics.sequencesSkipped.toInt(),
                    lateDropCount = diagnostics.lateRejections.toInt(),
                    underrunCount = diagnostics.ringUnderruns.toInt(),
                    concealedPacketCount = diagnostics.concealedPackets.toInt(),
                    metricsSummary = summarizeMetrics(),
                )
            }
            refreshListenerDiagnostics()
            delay(LISTENER_DIAGNOSTICS_POLL_MS)
        }
    }
}

/** Stops Rust playback and the native output for the discovered-session path. */
internal fun MainViewModel.stopListenerPlayback() {
    playbackJob?.cancel()
    playbackJob = null
    listenerPlaybackControlExecutor.execute { stopListenerPlaybackNow() }
}

private fun MainViewModel.stopListenerPlaybackNow() {
    val runtime = listenerPlayback ?: return
    listenerPlayback = null
    var firstFailure: Throwable? = null
    fun capture(block: () -> Unit) {
        try {
            block()
        } catch (error: Throwable) {
            val first = firstFailure
            if (first == null) firstFailure = error else first.addSuppressed(error)
        }
    }
    capture { listenerTransportController.detachPlayback() }
    capture { runtime.stop() }
    capture { OboeBridge.nativeOboeClose() }
    capture { runtime.close() }
    firstFailure?.let { error ->
        logger.w("playback.stop_failed", error.message ?: "playback failed to stop cleanly")
        _uiState.value = _uiState.value.copy(
            listenerPlaybackState = PlaybackState.ERROR,
            lastError = error.message ?: "playback failed to stop cleanly",
        )
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
