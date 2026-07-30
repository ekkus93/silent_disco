#!/usr/bin/env python3
"""Route Android host lifecycle and admission through the authoritative Rust actor."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def write(path: str, content: str) -> None:
    target = ROOT / path
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(content, encoding="utf-8")


def replace_once(path: str, label: str, old: str, new: str) -> None:
    content = read(path)
    count = content.count(old)
    if count != 1:
        raise SystemExit(f"{path} [{label}]: expected one match, found {count}")
    write(path, content.replace(old, new))


def remove_between(path: str, label: str, start: str, end: str) -> None:
    content = read(path)
    start_index = content.find(start)
    end_index = content.find(end, start_index)
    if start_index < 0 or end_index < 0:
        raise SystemExit(f"{path} [{label}]: boundary not found")
    write(path, content[:start_index] + content[end_index:])


HOST_CORE_CONTROLLER = r'''package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiAppRole
import com.ekkus.silentdisco.core.uniffi.FfiCoreHandle
import com.ekkus.silentdisco.core.uniffi.FfiCoreNotification
import com.ekkus.silentdisco.core.uniffi.FfiCoreObserver
import com.ekkus.silentdisco.core.uniffi.FfiCoreSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiDeliveryReport
import com.ekkus.silentdisco.core.uniffi.FfiHostDraft
import com.ekkus.silentdisco.core.uniffi.FfiJoinRequestInput
import com.ekkus.silentdisco.core.uniffi.FfiListenerSummary
import com.ekkus.silentdisco.core.uniffi.FfiPlatformCompletion
import com.ekkus.silentdisco.core.uniffi.FfiPlaybackState
import com.ekkus.silentdisco.core.uniffi.FfiTransportState
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeout

/** Android-facing command and event port for the authoritative Rust host actor. */
interface HostCoreController : AutoCloseable {
    val snapshots: StateFlow<FfiCoreSnapshot?>
    val notifications: Flow<FfiCoreNotification>

    suspend fun configureAndCreate(draft: FfiHostDraft)
    suspend fun approveJoin(requestId: String, rememberForFuture: Boolean)
    suspend fun rejectJoin(requestId: String)
    suspend fun removeListener(listenerId: String)
    suspend fun endHostSession()
    suspend fun retryRecoverableFailure()

    fun submitJoinRequest(request: FfiJoinRequestInput)
    fun submitListenerConnected(listener: FfiListenerSummary)
    fun submitListenerDisconnected(deviceId: String)
    fun transportStateChanged(state: FfiTransportState)
    fun transportFailed(message: String, retryable: Boolean)
    fun transportDeliveryCompleted(operationId: String, report: FfiDeliveryReport)
    fun playbackStateChanged(state: FfiPlaybackState)
    fun platformOperationSucceeded(operationId: String, completion: FfiPlatformCompletion)
    fun platformOperationFailed(operationId: String, message: String, retryable: Boolean)
    fun settingsSaved(operationId: String)
    fun trustedDeviceUpdated(operationId: String, deviceId: String)
    fun storageOperationFailed(operationId: String, message: String, retryable: Boolean)
}

/** UniFFI implementation that serializes revision-bearing commands. */
class UniFfiHostCoreController(localDeviceId: String) : HostCoreController {
    private val closed = AtomicBoolean(false)
    private val commandMutex = Mutex()
    private val _snapshots = MutableStateFlow<FfiCoreSnapshot?>(null)
    private val notificationChannel = Channel<FfiCoreNotification>(Channel.UNLIMITED)

    private val observer = object : FfiCoreObserver {
        override fun onNotification(notification: FfiCoreNotification) {
            when (notification) {
                is FfiCoreNotification.Snapshot -> _snapshots.value = notification.snapshot
                else -> check(notificationChannel.trySend(notification).isSuccess) {
                    "Rust host notification channel is closed"
                }
            }
        }
    }

    private val handle = FfiCoreHandle.open(localDeviceId, observer)

    override val snapshots: StateFlow<FfiCoreSnapshot?> = _snapshots
    override val notifications: Flow<FfiCoreNotification> = notificationChannel.receiveAsFlow()

    init {
        _snapshots.value = handle.currentSnapshot()
    }

    override suspend fun configureAndCreate(draft: FfiHostDraft) = commandMutex.withLock {
        var snapshot = currentSnapshot()
        if (snapshot.selectedRole != FfiAppRole.HOST) {
            snapshot = awaitCommandSnapshot(
                handle.selectRole(snapshot.revision, FfiAppRole.HOST).acceptedAtRevision,
            )
        }
        if (snapshot.hostDraft != draft) {
            snapshot = awaitCommandSnapshot(
                handle.updateHostDraft(snapshot.revision, draft).acceptedAtRevision,
            )
        }
        handle.createHostSession(snapshot.revision)
    }

    override suspend fun approveJoin(requestId: String, rememberForFuture: Boolean) =
        submitRevisionCommand { revision ->
            handle.approveJoin(revision, requestId, rememberForFuture)
        }

    override suspend fun rejectJoin(requestId: String) = submitRevisionCommand { revision ->
        handle.rejectJoin(revision, requestId)
    }

    override suspend fun removeListener(listenerId: String) = submitRevisionCommand { revision ->
        handle.removeListener(revision, listenerId)
    }

    override suspend fun endHostSession() = submitRevisionCommand { revision ->
        handle.endHostSession(revision)
    }

    override suspend fun retryRecoverableFailure() = submitRevisionCommand { revision ->
        handle.retryRecoverableFailure(revision)
    }

    override fun submitJoinRequest(request: FfiJoinRequestInput) = handle.submitJoinRequest(request)

    override fun submitListenerConnected(listener: FfiListenerSummary) =
        handle.submitListenerConnected(listener)

    override fun submitListenerDisconnected(deviceId: String) =
        handle.submitListenerDisconnected(deviceId, null)

    override fun transportStateChanged(state: FfiTransportState) = handle.transportStateChanged(state)

    override fun transportFailed(message: String, retryable: Boolean) =
        handle.transportFailed(message, retryable)

    override fun transportDeliveryCompleted(operationId: String, report: FfiDeliveryReport) =
        handle.transportDeliveryCompleted(operationId, report)

    override fun playbackStateChanged(state: FfiPlaybackState) = handle.playbackStateChanged(state)

    override fun platformOperationSucceeded(
        operationId: String,
        completion: FfiPlatformCompletion,
    ) = handle.platformOperationSucceeded(operationId, completion)

    override fun platformOperationFailed(
        operationId: String,
        message: String,
        retryable: Boolean,
    ) = handle.platformOperationFailed(operationId, message, retryable)

    override fun settingsSaved(operationId: String) = handle.settingsSaved(operationId)

    override fun trustedDeviceUpdated(operationId: String, deviceId: String) =
        handle.trustedDeviceUpdated(operationId, deviceId)

    override fun storageOperationFailed(
        operationId: String,
        message: String,
        retryable: Boolean,
    ) = handle.storageOperationFailed(operationId, message, retryable)

    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        runCatching { handle.shutdown() }
        notificationChannel.close()
        handle.close()
    }

    private suspend fun submitRevisionCommand(
        command: (ULong) -> Unit,
    ) = commandMutex.withLock {
        command(currentSnapshot().revision)
    }

    private fun currentSnapshot(): FfiCoreSnapshot =
        _snapshots.value ?: handle.currentSnapshot().also { _snapshots.value = it }

    private suspend fun awaitCommandSnapshot(acceptedAtRevision: ULong): FfiCoreSnapshot =
        withTimeout(SNAPSHOT_TIMEOUT_MS) {
            snapshots.filterNotNull().first { it.revision > acceptedAtRevision }
        }

    private companion object {
        const val SNAPSHOT_TIMEOUT_MS = 5_000L
    }
}
'''


MAIN_VIEW_MODEL_RUST_HOST = r'''package com.ekkus.silentdisco.app

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
import com.ekkus.silentdisco.core.protocol.ControlMessage
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.rust.HostCoreController
import com.ekkus.silentdisco.core.uniffi.FfiAppRole
import com.ekkus.silentdisco.core.uniffi.FfiApprovalMode
import com.ekkus.silentdisco.core.uniffi.FfiAudioSource
import com.ekkus.silentdisco.core.uniffi.FfiCoreNotification
import com.ekkus.silentdisco.core.uniffi.FfiCoreSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiDeliveryReport
import com.ekkus.silentdisco.core.uniffi.FfiHostDraft
import com.ekkus.silentdisco.core.uniffi.FfiHostLifecycle
import com.ekkus.silentdisco.core.uniffi.FfiJoinRequestInput
import com.ekkus.silentdisco.core.uniffi.FfiListenerSummary
import com.ekkus.silentdisco.core.uniffi.FfiPlatformCompletion
import com.ekkus.silentdisco.core.uniffi.FfiPlatformEffect
import com.ekkus.silentdisco.core.uniffi.FfiPlaybackState
import com.ekkus.silentdisco.core.uniffi.FfiStorageEffect
import com.ekkus.silentdisco.core.uniffi.FfiTransportEffect
import com.ekkus.silentdisco.core.uniffi.FfiTransportState
import com.ekkus.silentdisco.core.uniffi.FfiTrustState
import com.ekkus.silentdisco.core.uniffi.FfiTuningSettings
import com.ekkus.silentdisco.core.transport.BleAdvertisement
import java.security.MessageDigest
import java.util.UUID
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.launch

internal fun MainViewModel.ensureRustHostCore(): HostCoreController {
    hostCoreController?.let { return it }
    val controller = hostCoreFactory(ANDROID_HOST_DEVICE_ID)
    hostCoreController = controller
    viewModelScope.launch {
        controller.snapshots.filterNotNull().collect(::applyRustHostSnapshot)
    }
    viewModelScope.launch {
        controller.notifications.collect { notification ->
            executeRustHostNotification(controller, notification)
        }
    }
    return controller
}

internal fun MainViewModel.createRustHostSession() {
    if (!requirePersistenceReady("start a host session")) return
    val controller = ensureRustHostCore()
    val draft = _uiState.value.hostForm.toFfiHostDraft(_uiState.value.tuningSettings)
    _uiState.value = _uiState.value.copy(
        lastMessage = "Starting host session…",
        lastError = null,
    )
    viewModelScope.launch {
        runCatching { controller.configureAndCreate(draft) }
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
            trustState = FfiTrustState.SESSION_ONLY,
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

internal fun MainViewModel.trustRustListener(listenerId: String) {
    val displayName = _uiState.value.approvedListeners
        .firstOrNull { it.deviceId == listenerId }
        ?.displayName
        ?: listenerId
    viewModelScope.launch {
        runCatching { domainStore.trustDevice(listenerId, displayName) }
            .onSuccess {
                ensureRustHostCore().trustedDeviceUpdated(
                    operationId = "manual-trust-${UUID.randomUUID()}",
                    deviceId = listenerId,
                )
            }
            .onFailure(::reportTrustedListenerPersistenceFailure)
    }
}

internal fun MainViewModel.endRustHostSession() {
    viewModelScope.launch {
        runCatching { ensureRustHostCore().endHostSession() }
            .onFailure { reportRustHostCommandFailure("end host session", it) }
    }
}

internal fun MainViewModel.submitRustJoinRequest(message: ControlMessage.JoinRequest) {
    if (message.sessionId != currentSessionId) return
    runCatching {
        ensureRustHostCore().submitJoinRequest(
            FfiJoinRequestInput(
                requestId = "${message.device.deviceId}-${message.sessionId.value}",
                deviceId = message.device.deviceId,
                displayName = message.device.displayName,
                trustState = FfiTrustState.UNKNOWN,
                inviteCode = message.inviteCode,
                receivedAtMs = SystemClock.elapsedRealtime().toULong(),
            ),
        )
    }.onFailure { reportRustHostCommandFailure("submit join request", it) }
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
    _uiState.value = _uiState.value.copy(
        hostState = hostState,
        pendingJoinRequests = pending,
        approvedListeners = listeners,
        hostPlaybackState = playbackState,
        lastError = snapshot.lastError?.message,
        lastMessage = hostState.presentationMessage(_uiState.value.lastMessage),
    )
    diagnosticsStore.updateHost {
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

private suspend fun MainViewModel.executeRustHostNotification(
    controller: HostCoreController,
    notification: FfiCoreNotification,
) {
    when (notification) {
        is FfiCoreNotification.Snapshot -> applyRustHostSnapshot(notification.snapshot)
        is FfiCoreNotification.PlatformEffect -> executeRustPlatformEffect(controller, notification.effect)
        is FfiCoreNotification.TransportEffect -> executeRustTransportEffect(controller, notification.effect)
        is FfiCoreNotification.StorageEffect -> executeRustStorageEffect(controller, notification.effect)
        is FfiCoreNotification.Error -> {
            logger.e("rust.host", notification.error.message)
            _uiState.value = _uiState.value.copy(lastError = notification.error.message)
        }
        is FfiCoreNotification.Diagnostic -> {
            logger.i("rust.host.${notification.diagnostic.name}", notification.diagnostic.fields.joinToString())
        }
    }
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
    currentStreamId = StreamId("stream-${SystemClock.elapsedRealtime()}")
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
    _uiState.value = _uiState.value.copy(
        discoveredSessions = listOf(session),
        lastMessage = "Hosting ${session.name}",
        lastError = null,
    )
    diagnosticsStore.updateHost {
        it.copy(
            sessionId = session.id,
            streamState = PlaybackState.STOPPED,
            lastContactElapsedMs = SystemClock.elapsedRealtime(),
            lastError = null,
        )
    }
    controller.transportStateChanged(FfiTransportState.ADVERTISING)
    controller.platformOperationSucceeded(
        effect.operationId,
        FfiPlatformCompletion.AdvertisingStarted,
    )
}

private fun MainViewModel.stopAdvertisingForRust(
    controller: HostCoreController,
    effect: FfiPlatformEffect.StopAdvertising,
) {
    hostStreamJob?.cancel()
    playbackJob?.cancel()
    resyncJob?.cancel()
    playbackEngine.stop()
    bleService.stop()
    wifiDirectService.stop()
    controller.transportStateChanged(FfiTransportState.IDLE)
    controller.platformOperationSucceeded(
        effect.operationId,
        FfiPlatformCompletion.AdvertisingStopped,
    )
    currentSessionId = null
    currentStreamId = null
    latestPackets = emptyList()
    _uiState.value = _uiState.value.copy(
        discoveredSessions = emptyList(),
        lastMessage = "Session ended",
    )
}

private suspend fun MainViewModel.executeRustTransportEffect(
    controller: HostCoreController,
    effect: FfiTransportEffect,
) {
    val result = when (effect) {
        is FfiTransportEffect.DeliverJoinApproval -> wifiDirectService.sendControlToListener(
            effect.listenerId,
            ControlMessage.JoinApproval(
                version = 1,
                sessionId = SessionId(effect.sessionId),
                listenerId = effect.listenerId,
                trustedForFuture = effect.trustedForFuture,
            ),
        )
        is FfiTransportEffect.DeliverJoinRejection -> wifiDirectService.sendControlToListener(
            effect.listenerId,
            ControlMessage.JoinRejection(
                version = 1,
                sessionId = SessionId(effect.sessionId),
                listenerId = effect.listenerId,
                reason = effect.reasonCode,
            ),
        )
        is FfiTransportEffect.DisconnectListener -> wifiDirectService.sendControlToListener(
            effect.listenerId,
            ControlMessage.Disconnect(
                version = 1,
                sessionId = SessionId(effect.sessionId),
                listenerId = effect.listenerId,
                reason = effect.reasonCode,
            ),
        )
    }
    controller.transportDeliveryCompleted(
        operationId = when (effect) {
            is FfiTransportEffect.DeliverJoinApproval -> effect.operationId
            is FfiTransportEffect.DeliverJoinRejection -> effect.operationId
            is FfiTransportEffect.DisconnectListener -> effect.operationId
        },
        report = FfiDeliveryReport(
            intendedPeers = result.intendedPeerCount.toUInt(),
            successfulPeers = result.successCount.toUInt(),
            failedPeers = result.failureCount.toUInt(),
        ),
    )
    if (effect is FfiTransportEffect.DeliverJoinApproval && result.deliveredToTarget) {
        val displayName = _uiState.value.pendingJoinRequests
            .firstOrNull { it.listenerId == effect.listenerId }
            ?.listenerName
            ?: effect.listenerId
        controller.submitListenerConnected(
            FfiListenerSummary(
                deviceId = effect.listenerId,
                displayName = displayName,
                trustState = if (effect.trustedForFuture) {
                    FfiTrustState.TRUSTED
                } else {
                    FfiTrustState.SESSION_ONLY
                },
                transportState = FfiTransportState.CONNECTED,
                synchronization = null,
                lastContactMs = SystemClock.elapsedRealtime().toULong(),
                lastError = null,
            ),
        )
    }
}

private suspend fun MainViewModel.executeRustStorageEffect(
    controller: HostCoreController,
    effect: FfiStorageEffect,
) {
    when (effect) {
        is FfiStorageEffect.PersistSettings -> runCatching {
            domainStore.saveTuning(effect.settings.toRustStoredSettings())
        }.onSuccess {
            controller.settingsSaved(effect.operationId)
        }.onFailure { error ->
            controller.storageOperationFailed(
                effect.operationId,
                error.message ?: "Failed to persist settings",
                true,
            )
        }
        is FfiStorageEffect.PersistTrustedDevice -> runCatching {
            domainStore.trustDevice(effect.deviceId, effect.displayName)
        }.onSuccess {
            controller.trustedDeviceUpdated(effect.operationId, effect.deviceId)
        }.onFailure { error ->
            controller.storageOperationFailed(
                effect.operationId,
                error.message ?: "Failed to persist trusted device",
                true,
            )
        }
    }
}

private fun MainViewModel.reportRustHostCommandFailure(action: String, error: Throwable) {
    val message = error.message ?: "Failed to $action"
    logger.e("rust.host.command", message, error)
    _uiState.value = _uiState.value.copy(lastError = message)
}

internal fun HostFormState.toFfiHostDraft(tuning: TuningSettings): FfiHostDraft = FfiHostDraft(
    sessionName = sessionName,
    approvalMode = approvalMode.toFfiApprovalMode(),
    inviteCode = inviteCode.takeIf { approvalMode == ApprovalMode.INVITE_CODE },
    audioSource = selectedAudio?.let { audio ->
        FfiAudioSource(
            sourceId = opaqueAudioSourceId(audio.uri.toString()),
            displayName = audio.displayName,
            sizeBytes = audio.sizeBytes?.toULong(),
            durationMs = null,
        )
    },
    rememberApprovedDevices = rememberApprovedDevices,
    tuning = tuning.toFfiTuningSettings(),
)

private fun opaqueAudioSourceId(uri: String): String {
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

private fun FfiTuningSettings.toRustStoredSettings() =
    com.ekkus.silentdisco.core.rust.RustStoredTuningSettings(
        syncSampleWindow = syncSampleWindow.toInt(),
        syncCadenceMs = syncCadenceMs.toLong(),
        startupBufferMs = startupBufferMs.toLong(),
        latePacketThresholdMs = latePacketThresholdMs.toLong(),
        hardResyncThresholdMs = hardResyncThresholdMs.toLong(),
        syncDriftThresholdMs = syncDriftThresholdMs,
        scanWindowMs = scanWindowMs.toLong(),
        updatedAtMs = System.currentTimeMillis(),
    )

private fun ApprovalMode.toFfiApprovalMode(): FfiApprovalMode = when (this) {
    ApprovalMode.MANUAL -> FfiApprovalMode.MANUAL
    ApprovalMode.TRUSTED_DEVICES_PLACEHOLDER -> FfiApprovalMode.TRUSTED_DEVICES
    ApprovalMode.INVITE_CODE -> FfiApprovalMode.INVITE_CODE
}

private fun FfiApprovalMode.toAppApprovalMode(): ApprovalMode = when (this) {
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

private const val ANDROID_HOST_DEVICE_ID = "android-host-device"
'''


JOIN_APPROVAL_COMMAND = r'''package com.ekkus.silentdisco.app

import androidx.annotation.MainThread
import com.ekkus.silentdisco.core.model.JoinRequest

/** Dispatches an immutable per-request approval lifetime directly to Rust. */
@MainThread
internal fun MainViewModel.dispatchJoinApproval(
    request: JoinRequest,
    rememberForFuture: Boolean,
) {
    approveJoinRequest(request, rememberForFuture)
}
'''


INVITE_CODE_TEST = r'''package com.ekkus.silentdisco.app

import android.net.Uri
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.ekkus.silentdisco.core.uniffi.FfiApprovalMode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class InviteCodeValidationTest {
    @Test
    fun inviteCodeAndOpaqueAudioIdentityArePassedToRustWithoutKotlinValidation() {
        val draft = HostFormState(
            sessionName = "Invite session",
            approvalMode = ApprovalMode.INVITE_CODE,
            inviteCode = " 2468 ",
            selectedAudio = SelectedAudioFile(
                uri = Uri.parse("content://private/audio/42"),
                displayName = "set.wav",
                mimeType = "audio/wav",
                sizeBytes = 512,
            ),
        ).toFfiHostDraft(TuningSettings())

        assertEquals(FfiApprovalMode.INVITE_CODE, draft.approvalMode)
        assertEquals(" 2468 ", draft.inviteCode)
        assertFalse(draft.audioSource!!.sourceId.contains("content://"))
    }
}
'''


def write_new_files() -> None:
    write(
        "app/src/main/java/com/ekkus/silentdisco/core/rust/HostCoreController.kt",
        HOST_CORE_CONTROLLER,
    )
    write(
        "app/src/main/java/com/ekkus/silentdisco/app/MainViewModelRustHost.kt",
        MAIN_VIEW_MODEL_RUST_HOST,
    )
    write(
        "app/src/main/java/com/ekkus/silentdisco/app/JoinApprovalCommand.kt",
        JOIN_APPROVAL_COMMAND,
    )
    write(
        "app/src/test/java/com/ekkus/silentdisco/app/InviteCodeValidationTest.kt",
        INVITE_CODE_TEST,
    )
    stale_test = ROOT / "app/src/test/java/com/ekkus/silentdisco/app/JoinApprovalCommandTest.kt"
    if stale_test.exists():
        stale_test.unlink()


def patch_main_view_model() -> None:
    path = "app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt"
    replace_once(
        path,
        "host-core-imports",
        "import com.ekkus.silentdisco.core.rust.RustStoredTuningSettings\n",
        (
            "import com.ekkus.silentdisco.core.rust.HostCoreController\n"
            "import com.ekkus.silentdisco.core.rust.RustStoredTuningSettings\n"
            "import com.ekkus.silentdisco.core.rust.UniFfiHostCoreController\n"
        ),
    )
    replace_once(
        path,
        "host-core-factory",
        (
            "    internal val playbackEngine: PlaybackEngine = AudioTrackPlaybackEngine(),\n"
            "    internal val domainStore: AndroidRustDomainStore = AndroidRustDomainStore(application),\n"
            ") : AndroidViewModel(application) {\n"
        ),
        (
            "    internal val playbackEngine: PlaybackEngine = AudioTrackPlaybackEngine(),\n"
            "    internal val domainStore: AndroidRustDomainStore = AndroidRustDomainStore(application),\n"
            "    internal val hostCoreFactory: (String) -> HostCoreController = {\n"
            "        UniFfiHostCoreController(it)\n"
            "    },\n"
            ") : AndroidViewModel(application) {\n"
        ),
    )
    replace_once(
        path,
        "host-core-field",
        "    internal var pendingJoinRequestMessage: ControlMessage.JoinRequest? = null\n",
        (
            "    internal var pendingJoinRequestMessage: ControlMessage.JoinRequest? = null\n"
            "    internal var hostCoreController: HostCoreController? = null\n"
        ),
    )
    replace_once(
        path,
        "create-host-command",
        (
            "    internal fun validateHostForm(state: AppUiState): String? = HostSessionValidator.validate(state.hostForm)\n\n"
            "    fun createHostSession(): Boolean = createHostSessionImpl()\n"
        ),
        "    fun createHostSession() = createRustHostSession()\n",
    )
    remove_between(
        path,
        "demo-request-reducer",
        "    fun addDemoJoinRequest() {\n",
        "    fun approveJoinRequest(request: JoinRequest)",
    )
    replace_once(
        path,
        "approval-commands",
        (
            "    fun approveJoinRequest(request: JoinRequest) = approveJoinRequestImpl(request)\n\n"
            "    fun rejectJoinRequest(request: JoinRequest) = rejectJoinRequestImpl(request)\n"
        ),
        (
            "    fun addDemoJoinRequest() = submitDemoRustJoinRequest()\n\n"
            "    fun approveJoinRequest(\n"
            "        request: JoinRequest,\n"
            "        rememberForFuture: Boolean = _uiState.value.hostForm.rememberApprovedDevices,\n"
            "    ) = approveRustJoinRequest(request, rememberForFuture)\n\n"
            "    fun rejectJoinRequest(request: JoinRequest) = rejectRustJoinRequest(request)\n"
        ),
    )
    remove_between(
        path,
        "manual-trust-reducer",
        "    fun trustListener(listenerId: String) {\n",
        "    internal fun trustedListenerPersistenceMessage",
    )
    replace_once(
        path,
        "manual-trust-command",
        "    internal fun trustedListenerPersistenceMessage(error: Throwable): String =\n",
        (
            "    fun trustListener(listenerId: String) = trustRustListener(listenerId)\n\n"
            "    internal fun trustedListenerPersistenceMessage(error: Throwable): String =\n"
        ),
    )
    remove_between(
        path,
        "remove-listener-reducer",
        "    fun removeListener(listenerId: String) {\n",
        "    fun startHostPlayback()",
    )
    replace_once(
        path,
        "remove-listener-command",
        "    fun startHostPlayback() = startHostPlaybackImpl()\n",
        (
            "    fun removeListener(listenerId: String) = removeRustListener(listenerId)\n\n"
            "    fun startHostPlayback() = startHostPlaybackImpl()\n"
        ),
    )
    replace_once(
        path,
        "end-host-command",
        "    fun endSession() = endSessionImpl()\n",
        "    fun endSession() = endRustHostSession()\n",
    )
    replace_once(
        path,
        "close-rust-host",
        (
            "        bleService.stop()\n"
            "        wifiDirectService.stop()\n"
            "        runBlocking(Dispatchers.IO) { domainStore.close() }\n"
        ),
        (
            "        bleService.stop()\n"
            "        wifiDirectService.stop()\n"
            "        hostCoreController?.close()\n"
            "        runBlocking(Dispatchers.IO) { domainStore.close() }\n"
        ),
    )


def patch_host_actions() -> None:
    path = "app/src/main/java/com/ekkus/silentdisco/app/MainViewModelHostActions.kt"
    remove_between(
        path,
        "legacy-host-authority",
        "internal fun MainViewModel.createHostSessionImpl(): Boolean {\n",
        "internal fun MainViewModel.startHostPlaybackImpl()",
    )
    replace_once(
        path,
        "missing-session-failure",
        (
            "        _uiState.value = _uiState.value.copy(\n"
            "            hostState = HostLifecycleState.ERROR,\n"
            "            hostPlaybackState = PlaybackState.ERROR,\n"
            "            lastError = sessionError,\n"
            "        )\n"
        ),
        (
            "        _uiState.value = _uiState.value.copy(lastError = sessionError)\n"
            "        reportRustHostPlaybackState(PlaybackState.ERROR, sessionError)\n"
        ),
    )
    replace_once(
        path,
        "streaming-snapshot-authority",
        (
            "        _uiState.value = _uiState.value.copy(\n"
            "            hostState = HostLifecycleState.STREAMING,\n"
            "            hostPlaybackState = PlaybackState.PLAYING,\n"
            "            lastMessage = \"Host stream started via $backend\",\n"
            "            lastError = null,\n"
            "        )\n"
        ),
        (
            "        _uiState.value = _uiState.value.copy(\n"
            "            lastMessage = \"Host stream started via $backend\",\n"
            "            lastError = null,\n"
            "        )\n"
            "        reportRustHostPlaybackState(PlaybackState.PLAYING)\n"
        ),
    )
    replace_once(
        path,
        "decode-failure-authority",
        (
            "        _uiState.value = _uiState.value.copy(\n"
            "            hostState = HostLifecycleState.ERROR,\n"
            "            hostPlaybackState = PlaybackState.ERROR,\n"
            "            lastError = error.message ?: \"Failed to decode audio file\",\n"
            "        )\n"
        ),
        (
            "        val message = error.message ?: \"Failed to decode audio file\"\n"
            "        _uiState.value = _uiState.value.copy(lastError = message)\n"
            "        reportRustHostPlaybackState(PlaybackState.ERROR, message)\n"
        ),
    )
    replace_once(
        path,
        "pause-authority",
        (
            "    _uiState.value = _uiState.value.copy(\n"
            "        hostState = HostLifecycleState.PAUSED,\n"
            "        hostPlaybackState = PlaybackState.PAUSED,\n"
            "    )\n"
        ),
        "    reportRustHostPlaybackState(PlaybackState.PAUSED)\n",
    )
    replace_once(
        path,
        "stop-authority",
        (
            "    _uiState.value = _uiState.value.copy(\n"
            "        hostState = HostLifecycleState.READY,\n"
            "        hostPlaybackState = PlaybackState.STOPPED,\n"
            "    )\n"
        ),
        "    reportRustHostPlaybackState(PlaybackState.STOPPED)\n",
    )
    content = read(path)
    marker = "internal fun MainViewModel.endSessionImpl() {\n"
    index = content.find(marker)
    if index < 0:
        raise SystemExit(f"{path} [legacy-end-reducer]: marker not found")
    write(path, content[:index])


def patch_host_playback() -> None:
    path = "app/src/main/java/com/ekkus/silentdisco/app/MainViewModelHostPlayback.kt"
    replace_once(
        path,
        "repeated-send-failure",
        (
            "                    _uiState.value = _uiState.value.copy(\n"
            "                        hostState = HostLifecycleState.ERROR,\n"
            "                        hostPlaybackState = PlaybackState.ERROR,\n"
            "                        lastError = message,\n"
            "                    )\n"
        ),
        (
            "                    _uiState.value = _uiState.value.copy(lastError = message)\n"
            "                    reportRustHostPlaybackState(PlaybackState.ERROR, message)\n"
        ),
    )
    replace_once(
        path,
        "eof-authority",
        (
            "            _uiState.value = _uiState.value.copy(\n"
            "                hostState = HostLifecycleState.READY,\n"
            "                hostPlaybackState = PlaybackState.STOPPED,\n"
            "                lastMessage = \"Reached end of file\",\n"
            "            )\n"
        ),
        (
            "            _uiState.value = _uiState.value.copy(lastMessage = \"Reached end of file\")\n"
            "            reportRustHostPlaybackState(PlaybackState.STOPPED)\n"
        ),
    )
    replace_once(
        path,
        "engine-failure-authority",
        (
            "        _uiState.value = _uiState.value.copy(\n"
            "            hostState = HostLifecycleState.ERROR,\n"
            "            hostPlaybackState = PlaybackState.ERROR,\n"
            "            lastError = message,\n"
            "        )\n"
        ),
        (
            "        _uiState.value = _uiState.value.copy(lastError = message)\n"
            "        reportRustHostPlaybackState(PlaybackState.ERROR, message)\n"
        ),
    )


def patch_transport() -> None:
    path = "app/src/main/java/com/ekkus/silentdisco/app/MainViewModelTransport.kt"
    replace_once(
        path,
        "transport-state-fact",
        (
            "                refreshDiscoveredSessions()\n"
            "                handleTransportSnapshot(snapshot)\n"
        ),
        (
            "                refreshDiscoveredSessions()\n"
            "                reportRustHostTransportState(snapshot.state)\n"
            "                handleTransportSnapshot(snapshot)\n"
        ),
    )
    replace_once(
        path,
        "host-transport-failure",
        (
            "            TransportSnapshotRole.HOST_FAILURE -> {\n"
            "                _uiState.value = _uiState.value.copy(\n"
            "                    hostState = HostLifecycleState.ERROR,\n"
            "                    hostPlaybackState = if (_uiState.value.hostPlaybackState == PlaybackState.PLAYING) {\n"
            "                        PlaybackState.ERROR\n"
            "                    } else {\n"
            "                        _uiState.value.hostPlaybackState\n"
            "                    },\n"
            "                    lastError = errorMessage,\n"
            "                )\n"
            "                diagnosticsStore.updateHost {\n"
            "                    it.copy(lastError = errorMessage, metricsSummary = summarizeMetrics())\n"
            "                }\n"
            "                refreshHostDiagnostics()\n"
            "            }\n"
        ),
        (
            "            TransportSnapshotRole.HOST_FAILURE -> {\n"
            "                reportRustHostTransportFailure(errorMessage, snapshot.lastError.retryable)\n"
            "            }\n"
        ),
    )
    replace_once(
        path,
        "ble-host-failure",
        (
            "        wifiDirectService.stop()\n"
            "        _uiState.value = _uiState.value.copy(\n"
            "            hostState = HostLifecycleState.ERROR,\n"
            "            hostPlaybackState = if (_uiState.value.hostPlaybackState == PlaybackState.PLAYING) {\n"
            "                PlaybackState.ERROR\n"
            "            } else {\n"
            "                _uiState.value.hostPlaybackState\n"
            "            },\n"
            "            lastError = message,\n"
            "        )\n"
            "        diagnosticsStore.updateHost {\n"
            "            it.copy(lastError = message, metricsSummary = summarizeMetrics())\n"
            "        }\n"
            "        refreshHostDiagnostics()\n"
        ),
        (
            "        wifiDirectService.stop()\n"
            "        reportRustHostTransportFailure(message, retryable = true)\n"
        ),
    )
    remove_between(
        path,
        "kotlin-invite-validator",
        "    internal fun MainViewModel.joinRejectionReason(message: ControlMessage.JoinRequest): String? {\n",
        "    internal fun MainViewModel.handleJoinRequestMessage",
    )
    content = read(path)
    start = content.find("    internal fun MainViewModel.handleJoinRequestMessage(message: ControlMessage.JoinRequest) {\n")
    end = content.find("    internal fun MainViewModel.handleJoinApprovalMessage", start)
    if start < 0 or end < 0:
        raise SystemExit(f"{path} [join-request-routing]: boundary not found")
    replacement = (
        "    internal fun MainViewModel.handleJoinRequestMessage(message: ControlMessage.JoinRequest) {\n"
        "        submitRustJoinRequest(message)\n"
        "    }\n\n"
    )
    write(path, content[:start] + replacement + content[end:])


def patch_app_state() -> None:
    path = "app/src/main/java/com/ekkus/silentdisco/app/AppState.kt"
    content = read(path)
    marker = "\ninternal object HostSessionValidator {\n"
    index = content.find(marker)
    if index < 0:
        raise SystemExit(f"{path} [host-validator]: marker not found")
    write(path, content[:index] + "\n")


def patch_workflow_navigation() -> None:
    path = "app/src/main/java/com/ekkus/silentdisco/app/WorkflowViewModel.kt"
    replace_once(
        path,
        "host-lifecycle-import",
        "import com.ekkus.silentdisco.core.model.JoinRequest\n",
        (
            "import com.ekkus.silentdisco.core.model.HostLifecycleState\n"
            "import com.ekkus.silentdisco.core.model.JoinRequest\n"
        ),
    )
    replace_once(
        path,
        "host-navigation-flag",
        "    private var playbackNavigationConsumed = false\n",
        (
            "    private var playbackNavigationConsumed = false\n"
            "    private var hostNavigationConsumed = false\n"
        ),
    )
    replace_once(
        path,
        "host-navigation-from-snapshot",
        (
            "        if (shouldNavigateToListenerPlayback(state, playbackNavigationConsumed)) {\n"
            "            playbackNavigationConsumed = true\n"
            "            emit(AppUiEffect.NavigateListenerPlayback)\n"
            "        }\n"
        ),
        (
            "        if (shouldNavigateToListenerPlayback(state, playbackNavigationConsumed)) {\n"
            "            playbackNavigationConsumed = true\n"
            "            emit(AppUiEffect.NavigateListenerPlayback)\n"
            "        }\n\n"
            "        if (state.hostState in hostNavigationResetStates) {\n"
            "            hostNavigationConsumed = false\n"
            "        }\n"
            "        if (!hostNavigationConsumed && state.hostState in hostDashboardStates) {\n"
            "            hostNavigationConsumed = true\n"
            "            emit(AppUiEffect.NavigateHostDashboard)\n"
            "        }\n"
        ),
    )
    replace_once(
        path,
        "host-navigation-sets",
        (
            "    private companion object {\n"
            "        val playbackResetStates = setOf(\n"
        ),
        (
            "    private companion object {\n"
            "        val hostDashboardStates = setOf(\n"
            "            HostLifecycleState.ADVERTISING,\n"
            "            HostLifecycleState.WAITING_FOR_LISTENERS,\n"
            "            HostLifecycleState.READY,\n"
            "            HostLifecycleState.STREAMING,\n"
            "            HostLifecycleState.PAUSED,\n"
            "        )\n"
            "        val hostNavigationResetStates = setOf(\n"
            "            HostLifecycleState.IDLE,\n"
            "            HostLifecycleState.ERROR,\n"
            "        )\n"
            "        val playbackResetStates = setOf(\n"
        ),
    )


def patch_silent_disco_app() -> None:
    path = "app/src/main/java/com/ekkus/silentdisco/app/SilentDiscoApp.kt"
    replace_once(
        path,
        "async-host-navigation",
        "workflowViewModel.onHostSessionCreationResult(viewModel.createHostSession())",
        "viewModel.createHostSession()",
    )


def main() -> None:
    write_new_files()
    patch_main_view_model()
    patch_host_actions()
    patch_host_playback()
    patch_transport()
    patch_app_state()
    patch_workflow_navigation()
    patch_silent_disco_app()


if __name__ == "__main__":
    main()
