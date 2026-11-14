package com.ekkus.silentdisco.app

import android.os.SystemClock
import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.JoinApprovalState
import com.ekkus.silentdisco.core.model.JoinRequest
import com.ekkus.silentdisco.core.model.ListenerInfo
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.model.TrustState
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.app.controller.HostSessionCallbacks
import com.ekkus.silentdisco.core.rust.HostCoreController
import com.ekkus.silentdisco.core.rust.RustStoredTuningSettings
import com.ekkus.silentdisco.core.uniffi.FfiApprovalMode
import com.ekkus.silentdisco.core.uniffi.FfiAudioSource
import com.ekkus.silentdisco.core.uniffi.FfiCoreNotification
import com.ekkus.silentdisco.core.uniffi.FfiCoreSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiHostDraft
import com.ekkus.silentdisco.core.uniffi.FfiHostLifecycle
import com.ekkus.silentdisco.core.uniffi.FfiQueueEntry
import com.ekkus.silentdisco.core.uniffi.FfiJoinRequestInput
import com.ekkus.silentdisco.core.uniffi.FfiListenerSummary
import com.ekkus.silentdisco.core.uniffi.FfiPlatformCompletion
import com.ekkus.silentdisco.core.uniffi.FfiPlatformEffect
import com.ekkus.silentdisco.core.uniffi.FfiPlaybackState
import com.ekkus.silentdisco.core.uniffi.FfiTransportState
import com.ekkus.silentdisco.core.uniffi.FfiTrustState
import com.ekkus.silentdisco.core.uniffi.FfiTuningSettings
import com.ekkus.silentdisco.core.transport.BleAdvertisement
import com.ekkus.silentdisco.core.transport.TransportSnapshot
import java.security.MessageDigest
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

internal fun MainViewModel.ensureRustHostCore(): HostCoreController = hostSession.ensureCore()

/** Called from the Wi-Fi Direct snapshot collector; see `HostSessionController.completeAdvertising`. */
internal suspend fun MainViewModel.completeRustHostAdvertising(snapshot: TransportSnapshot) =
    hostSession.completeAdvertising(snapshot)

/** How the host session reads the UI state and hands results back to it. */
internal fun MainViewModel.hostSessionCallbacks() = object : HostSessionCallbacks {
    override fun onSnapshot(snapshot: FfiCoreSnapshot) = applyRustHostSnapshot(snapshot)
    override suspend fun executePlatformEffect(core: HostCoreController, effect: FfiPlatformEffect) =
        executeRustPlatformEffect(core, effect)
    override fun onCoreError(message: String) {
        _uiState.value = _uiState.value.copy(lastError = message)
    }
    override fun recoverNotificationFailure(
        core: HostCoreController,
        notification: FfiCoreNotification,
        error: Exception,
    ) = recoverRustHostNotificationFailure(core, notification, error)
    override fun commandFailed(action: String, error: Throwable) = reportRustHostCommandFailure(action, error)
    override fun pendingJoinDisplayName(listenerId: String): String? =
        _uiState.value.pendingJoinRequests.firstOrNull { it.listenerId == listenerId }?.listenerName
    override suspend fun saveTuning(settings: RustStoredTuningSettings) = persistence.saveTuning(settings)
}

internal fun MainViewModel.createRustHostSession() {
    if (!requirePersistenceReady("start a host session")) return
    val controller = ensureRustHostCore()
    val hostForm = _uiState.value.hostForm
    val setupAudio = listOfNotNull(hostForm.selectedAudio) + hostForm.additionalAudio
    val draft = hostForm.toFfiHostDraft(_uiState.value.tuningSettings)
    val additionalEntries = hostForm.additionalAudio.mapIndexed { index, audio ->
        FfiQueueEntry(
            entryId = "android-entry-${UUID.randomUUID()}-${index + 1}",
            source = audio.toFfiAudioSource(),
        )
    }
    _uiState.value = _uiState.value.copy(
        lastMessage = "Starting host session…",
        lastError = null,
    )
    viewModelScope.launch {
        val newlyStaged = mutableSetOf<String>()
        val preparation = runCatching {
            withContext(Dispatchers.IO) {
                setupAudio.distinctBy { stableOpaqueAudioSourceId(it.uri.toString()) }.forEach { audio ->
                    val sourceId = stableOpaqueAudioSourceId(audio.uri.toString())
                    if (!hostPlayback.staging.isStaged(sourceId)) newlyStaged += sourceId
                    stageAndValidateHostSource(audio)
                }
            }
        }
        if (preparation.isFailure) {
            hostPlayback.staging.discard(newlyStaged)
            val error = preparation.exceptionOrNull() ?: IllegalStateException("Audio preparation failed")
            _uiState.value = _uiState.value.copy(lastError = error.message ?: "Could not prepare selected songs")
            return@launch
        }
        runCatching { controller.configureAndCreate(draft, additionalEntries) }
            .onFailure { reportRustHostCommandFailure("start host session", it) }
    }
}

internal fun MainViewModel.submitDemoRustJoinRequest() {
    val sessionId = currentSessionId?.value ?: return
    ensureRustHostCore().submitJoinRequest(
        FfiJoinRequestInput(
            requestId = UUID.randomUUID().toString(),
            deviceId = UUID.randomUUID().toString(),
            displayName = "Listener ${_uiState.value.pendingJoinRequests.size + 1}",
            inviteCode = null,
            receivedAtMs = SystemClock.elapsedRealtime().toULong(),
        ),
    )
    logger.i("approval.demo", "Submitted demo join request to Rust for $sessionId")
}

internal fun MainViewModel.approveRustJoinRequest(
    request: JoinRequest,
    rememberForFuture: Boolean,
) {
    viewModelScope.launch {
        runCatching {
            ensureRustHostCore().approveJoin(request.requestId, rememberForFuture)
        }.onFailure { reportRustHostCommandFailure("approve join request", it) }
    }
}

internal fun MainViewModel.rejectRustJoinRequest(request: JoinRequest) {
    viewModelScope.launch {
        runCatching { ensureRustHostCore().rejectJoin(request.requestId) }
            .onFailure { reportRustHostCommandFailure("reject join request", it) }
    }
}

internal fun MainViewModel.removeRustListener(listenerId: String) {
    viewModelScope.launch {
        runCatching { ensureRustHostCore().removeListener(listenerId) }
            .onFailure { reportRustHostCommandFailure("remove listener", it) }
    }
}

internal fun MainViewModel.endRustHostSession() {
    viewModelScope.launch {
        runCatching { ensureRustHostCore().endHostSession() }
            .onSuccess {
                releaseLiveQueueAudioGrants()
                withContext(Dispatchers.IO) { releaseStagedHostSources() }
            }
            .onFailure { reportRustHostCommandFailure("end host session", it) }
    }
}

internal fun MainViewModel.reportRustHostTransportState(state: TransportConnectionState) {
    val controller = hostCoreController ?: return
    runCatching { controller.transportStateChanged(state.toFfiTransportState()) }
        .onFailure { reportRustHostCommandFailure("report transport state", it) }
}

internal fun MainViewModel.reportRustHostTransportFailure(message: String, retryable: Boolean) {
    val controller = hostCoreController ?: return
    runCatching { controller.transportFailed(message, retryable) }
        .onFailure { reportRustHostCommandFailure("report transport failure", it) }
}

internal fun MainViewModel.reportRustHostPlaybackState(
    state: PlaybackState,
    errorMessage: String? = null,
) {
    val controller = hostCoreController ?: return
    runCatching { controller.playbackStateChanged(state.toFfiPlaybackState()) }
        .onFailure { reportRustHostCommandFailure("report playback state", it) }
    if (errorMessage != null) {
        _uiState.value = _uiState.value.copy(lastError = errorMessage)
    }
}

internal fun MainViewModel.reportRustHostPlaybackFailure(message: String) {
    val controller = hostCoreController ?: return
    runCatching { controller.playbackFailed(message) }
        .onFailure { reportRustHostCommandFailure("report playback failure", it) }
}

private fun MainViewModel.applyRustHostSnapshot(snapshot: FfiCoreSnapshot) {
    val sessionId = currentSessionId?.value ?: _uiState.value.hostDiagnostics.sessionId
    val pending = snapshot.pendingJoinRequests.map { request ->
        JoinRequest(
            requestId = request.requestId,
            sessionId = sessionId,
            listenerId = request.deviceId,
            listenerName = request.displayName,
            inviteCode = null,
            requestedAtMs = request.receivedAtMs.toLong(),
        )
    }
    val listeners = snapshot.listeners.map { listener -> listener.toAppListener(snapshot.playbackState) }
    val hostState = snapshot.hostLifecycle.toAppHostLifecycle()
    val playbackState = snapshot.playbackState.toAppPlaybackState()
    val queue = snapshot.hostQueue.map { entry ->
        HostQueueItem(
            entryId = entry.entryId,
            sourceId = entry.source.sourceId,
            displayName = entry.source.displayName,
            byteLength = entry.source.sizeBytes?.toLong(),
            durationMs = entry.source.durationMs?.toLong(),
        )
    }
    _uiState.value = _uiState.value.copy(
        hostState = hostState,
        pendingJoinRequests = pending,
        approvedListeners = listeners,
        hostPlaybackState = playbackState,
        hostPlaybackPositionMs = snapshot.playbackPositionMs.toLong(),
        hostQueue = queue,
        lastError = snapshot.lastError?.message,
        lastMessage = hostState.presentationMessage(_uiState.value.lastMessage),
    )
    diagnostics.updateHost {
        it.copy(
            sessionId = sessionId,
            listenerCount = listeners.size,
            pendingJoinCount = pending.size,
            connectedListenerCount = listeners.count {
                listener -> listener.connectionState == TransportConnectionState.CONNECTED
            },
            streamState = playbackState,
            lastError = snapshot.lastError?.message,
            metricsSummary = summarizeMetrics(),
        )
    }
    refreshHostDiagnostics(streamState = playbackState, sessionId = sessionId)
}

private suspend fun MainViewModel.executeRustPlatformEffect(
    controller: HostCoreController,
    effect: FfiPlatformEffect,
) {
    when (effect) {
        is FfiPlatformEffect.StartAdvertising -> startAdvertisingForRust(controller, effect)
        is FfiPlatformEffect.StopAdvertising -> stopAdvertisingForRust(controller, effect)
        is FfiPlatformEffect.RequestCapabilities -> controller.platformOperationFailed(
            effect.operationId,
            "Android capability requests must be resolved before host creation",
            true,
        )
        is FfiPlatformEffect.StartDiscovery,
        is FfiPlatformEffect.StopDiscovery,
        is FfiPlatformEffect.EstablishNetwork,
        is FfiPlatformEffect.ReleaseNetwork,
        is FfiPlatformEffect.PrepareAudioSource,
        is FfiPlatformEffect.StartAudioOutput,
        is FfiPlatformEffect.StopAudioOutput,
        is FfiPlatformEffect.ShareDiagnostics,
        -> {
            val operationId = when (effect) {
                is FfiPlatformEffect.StartDiscovery -> effect.operationId
                is FfiPlatformEffect.StopDiscovery -> effect.operationId
                is FfiPlatformEffect.EstablishNetwork -> effect.operationId
                is FfiPlatformEffect.ReleaseNetwork -> effect.operationId
                is FfiPlatformEffect.PrepareAudioSource -> effect.operationId
                is FfiPlatformEffect.StartAudioOutput -> effect.operationId
                is FfiPlatformEffect.StopAudioOutput -> effect.operationId
                is FfiPlatformEffect.ShareDiagnostics -> effect.operationId
                else -> error("unreachable platform effect")
            }
            controller.platformOperationFailed(
                operationId,
                "Platform effect is outside Android host Block 12",
                false,
            )
        }
    }
}

private fun MainViewModel.startAdvertisingForRust(
    controller: HostCoreController,
    effect: FfiPlatformEffect.StartAdvertising,
) {
    if (!hasHostTransportPermissions()) {
        controller.platformOperationFailed(
            effect.operationId,
            "Missing nearby connectivity permissions for advertising",
            true,
        )
        return
    }
    val session = SessionInfo(
        id = effect.sessionId,
        name = effect.sessionName,
        hostDeviceName = effect.hostDeviceId,
        approvalMode = effect.approvalMode.toAppApprovalMode(),
        inviteCodeRequired = effect.approvalMode == FfiApprovalMode.INVITE_CODE,
    )
    currentSessionId = SessionId(effect.sessionId)
    hostPlayback.streamId = StreamId("stream-${SystemClock.elapsedRealtime()}")
    val bleResult = bleService.startAdvertising(
        BleAdvertisement(
            sessionId = session.id,
            sessionName = session.name,
            hostName = session.hostDeviceName,
            approvalRequired = true,
            inviteCodeRequired = session.inviteCodeRequired,
        ),
    )
    if (!bleResult.started) {
        controller.platformOperationFailed(
            effect.operationId,
            bleResult.message ?: "BLE advertising could not start",
            true,
        )
        return
    }
    val wifiResult = runCatching { wifiDirectService.startHost(session) }
        .getOrElse { error ->
            bleService.stopAdvertising()
            controller.platformOperationFailed(
                effect.operationId,
                error.message ?: "Failed to start Wi-Fi Direct host",
                true,
            )
            return
        }
    if (!wifiResult.started) {
        bleService.stopAdvertising()
        controller.platformOperationFailed(
            effect.operationId,
            wifiResult.message ?: "Wi-Fi Direct host could not start",
            true,
        )
        return
    }
    hostSession.pendingStartAdvertisingOperationId = effect.operationId
    _uiState.value = _uiState.value.copy(
        discoveredSessions = listOf(session),
        lastMessage = "Hosting ${session.name}",
        lastError = null,
    )
    diagnostics.updateHost {
        it.copy(
            sessionId = session.id,
            streamState = PlaybackState.STOPPED,
            lastContactElapsedMs = SystemClock.elapsedRealtime(),
            lastError = null,
        )
    }
    // AdvertisingStarted is reported once the Wi-Fi Direct group actually
    // forms and its own address is known -- see completeRustHostAdvertising.
}

private suspend fun MainViewModel.stopAdvertisingForRust(
    controller: HostCoreController,
    effect: FfiPlatformEffect.StopAdvertising,
) {
    // Invalidate queued native binds before any suspending teardown work.
    hostSession.pendingStartAdvertisingOperationId = null
    hostPlayback.cancelStream()
    listenerPlaybackController.cancelStatusPoll()
    listenerPlaybackController.cancelResync()
    hostMonitor.stop()
    bleService.stop()
    wifiDirectService.stop()
    hostSession.joinRoutes.clear()
    // Native shutdown waits for in-flight UniFFI operations and the poller.
    // Never hold the Compose/main dispatcher while waiting for those threads.
    // Complete retirement even if the notification coroutine is cancelled.
    retireHostTransportOffMain(hostTransportController)
    controller.transportStateChanged(FfiTransportState.IDLE)
    controller.platformOperationSucceeded(
        effect.operationId,
        FfiPlatformCompletion.AdvertisingStopped,
    )
    currentSessionId = null
    hostPlayback.streamId = null
    // Ending the host session also ends any stream a listener role followed.
    listenerPlaybackController.streamId = null
    hostPlayback.latestPackets = emptyList()
    _uiState.value = _uiState.value.copy(
        discoveredSessions = emptyList(),
        lastMessage = "Session ended",
    )
}

internal fun MainViewModel.reportRustHostCommandFailure(action: String, error: Throwable) {
    val message = error.message ?: "Failed to $action"
    logger.e("rust.host.command", message, error)
    _uiState.value = _uiState.value.copy(lastError = message)
}

internal fun HostFormState.toFfiHostDraft(tuning: TuningSettings): FfiHostDraft = FfiHostDraft(
    sessionName = sessionName,
    approvalMode = approvalMode.toFfiApprovalMode(),
    inviteCode = inviteCode.takeIf { approvalMode == ApprovalMode.INVITE_CODE },
    audioSource = selectedAudio?.toFfiAudioSource(),
    rememberApprovedDevices = rememberApprovedDevices,
    tuning = tuning.toFfiTuningSettings(),
)

internal fun com.ekkus.silentdisco.core.model.SelectedAudioFile.toFfiAudioSource() = FfiAudioSource(
    sourceId = stableOpaqueAudioSourceId(uri.toString()),
    displayName = displayName,
    sizeBytes = sizeBytes?.toULong(),
    durationMs = null,
)

internal fun stableOpaqueAudioSourceId(uri: String): String {
    val digest = MessageDigest.getInstance("SHA-256").digest(uri.toByteArray())
    return "android-source-" + digest.take(16).joinToString("") { "%02x".format(it) }
}

private fun TuningSettings.toFfiTuningSettings(): FfiTuningSettings = FfiTuningSettings(
    syncSampleWindow = syncSampleWindow.toUInt(),
    syncCadenceMs = syncCadenceMs.toULong(),
    startupBufferMs = startupBufferMs.toULong(),
    latePacketThresholdMs = latePacketThresholdMs.toULong(),
    hardResyncThresholdMs = hardResyncThresholdMs.toULong(),
    syncDriftThresholdMs = syncDriftThresholdMs,
    scanWindowMs = scanWindowMs.toULong(),
)

internal fun ApprovalMode.toFfiApprovalMode(): FfiApprovalMode = when (this) {
    ApprovalMode.MANUAL -> FfiApprovalMode.MANUAL
    ApprovalMode.TRUSTED_DEVICES_PLACEHOLDER -> FfiApprovalMode.TRUSTED_DEVICES
    ApprovalMode.INVITE_CODE -> FfiApprovalMode.INVITE_CODE
}

internal fun FfiApprovalMode.toAppApprovalMode(): ApprovalMode = when (this) {
    FfiApprovalMode.MANUAL -> ApprovalMode.MANUAL
    FfiApprovalMode.TRUSTED_DEVICES -> ApprovalMode.TRUSTED_DEVICES_PLACEHOLDER
    FfiApprovalMode.INVITE_CODE -> ApprovalMode.INVITE_CODE
}

private fun FfiHostLifecycle.toAppHostLifecycle(): HostLifecycleState = when (this) {
    FfiHostLifecycle.IDLE -> HostLifecycleState.IDLE
    FfiHostLifecycle.CREATING_SESSION -> HostLifecycleState.CREATING_SESSION
    FfiHostLifecycle.ADVERTISING -> HostLifecycleState.ADVERTISING
    FfiHostLifecycle.WAITING_FOR_LISTENERS -> HostLifecycleState.WAITING_FOR_LISTENERS
    FfiHostLifecycle.READY -> HostLifecycleState.READY
    FfiHostLifecycle.STREAMING -> HostLifecycleState.STREAMING
    FfiHostLifecycle.PAUSED -> HostLifecycleState.PAUSED
    FfiHostLifecycle.ENDING_SESSION -> HostLifecycleState.ENDING_SESSION
    FfiHostLifecycle.ERROR -> HostLifecycleState.ERROR
}

private fun FfiPlaybackState.toAppPlaybackState(): PlaybackState = when (this) {
    FfiPlaybackState.STOPPED -> PlaybackState.STOPPED
    FfiPlaybackState.BUFFERING -> PlaybackState.BUFFERING
    FfiPlaybackState.READY -> PlaybackState.READY
    FfiPlaybackState.PLAYING -> PlaybackState.PLAYING
    FfiPlaybackState.PAUSED -> PlaybackState.PAUSED
    FfiPlaybackState.UNDERRUN -> PlaybackState.UNDERRUN
    FfiPlaybackState.ERROR -> PlaybackState.ERROR
}

private fun PlaybackState.toFfiPlaybackState(): FfiPlaybackState = when (this) {
    PlaybackState.STOPPED -> FfiPlaybackState.STOPPED
    PlaybackState.BUFFERING -> FfiPlaybackState.BUFFERING
    PlaybackState.READY -> FfiPlaybackState.READY
    PlaybackState.PLAYING -> FfiPlaybackState.PLAYING
    PlaybackState.PAUSED -> FfiPlaybackState.PAUSED
    PlaybackState.UNDERRUN -> FfiPlaybackState.UNDERRUN
    PlaybackState.ERROR -> FfiPlaybackState.ERROR
}

private fun TransportConnectionState.toFfiTransportState(): FfiTransportState = when (this) {
    TransportConnectionState.IDLE -> FfiTransportState.IDLE
    TransportConnectionState.DISCOVERING -> FfiTransportState.DISCOVERING
    TransportConnectionState.ADVERTISING -> FfiTransportState.ADVERTISING
    TransportConnectionState.CONNECTING -> FfiTransportState.CONNECTING
    TransportConnectionState.CONNECTED -> FfiTransportState.CONNECTED
    TransportConnectionState.RETRYING -> FfiTransportState.RETRYING
    TransportConnectionState.DISCONNECTED -> FfiTransportState.DISCONNECTED
    TransportConnectionState.FAILED -> FfiTransportState.FAILED
}

private fun FfiTransportState.toAppTransportState(): TransportConnectionState = when (this) {
    FfiTransportState.IDLE -> TransportConnectionState.IDLE
    FfiTransportState.DISCOVERING -> TransportConnectionState.DISCOVERING
    FfiTransportState.ADVERTISING -> TransportConnectionState.ADVERTISING
    FfiTransportState.CONNECTING -> TransportConnectionState.CONNECTING
    FfiTransportState.CONNECTED -> TransportConnectionState.CONNECTED
    FfiTransportState.RETRYING -> TransportConnectionState.RETRYING
    FfiTransportState.DISCONNECTED -> TransportConnectionState.DISCONNECTED
    FfiTransportState.FAILED -> TransportConnectionState.FAILED
}

private fun FfiListenerSummary.toAppListener(playbackState: FfiPlaybackState): ListenerInfo = ListenerInfo(
    deviceId = deviceId,
    displayName = displayName,
    joinState = JoinApprovalState.APPROVED,
    trustState = if (trustState == FfiTrustState.TRUSTED) {
        TrustState.TRUSTED_PLACEHOLDER
    } else {
        TrustState.SESSION_ONLY
    },
    connectionState = transportState.toAppTransportState(),
    listenerState = if (playbackState == FfiPlaybackState.PLAYING) {
        ListenerLifecycleState.PLAYING
    } else {
        ListenerLifecycleState.CONNECTING
    },
    syncQuality = synchronization?.confidence.toSyncQuality(),
)

private fun String?.toSyncQuality(): SyncQualityBadge = when (this?.uppercase()) {
    "POOR" -> SyncQualityBadge.POOR
    "FAIR" -> SyncQualityBadge.FAIR
    "GOOD" -> SyncQualityBadge.GOOD
    "EXCELLENT" -> SyncQualityBadge.EXCELLENT
    else -> SyncQualityBadge.UNKNOWN
}

private fun HostLifecycleState.presentationMessage(previous: String?): String? = when (this) {
    HostLifecycleState.CREATING_SESSION -> "Creating host session…"
    HostLifecycleState.ADVERTISING -> "Advertising host session…"
    HostLifecycleState.WAITING_FOR_LISTENERS -> "Waiting for listeners"
    HostLifecycleState.READY -> "Host session ready"
    HostLifecycleState.STREAMING -> "Host stream active"
    HostLifecycleState.PAUSED -> "Host stream paused"
    HostLifecycleState.ENDING_SESSION -> "Ending host session…"
    HostLifecycleState.IDLE,
    HostLifecycleState.ERROR,
    -> previous
}
