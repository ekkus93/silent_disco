package com.ekkus.silentdisco.app

import android.app.Application
import android.net.Uri
import android.os.SystemClock
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.BuildConfig
import com.ekkus.silentdisco.core.audio.AudioDecodeResult
import com.ekkus.silentdisco.core.audio.AudioFileAccessException
import com.ekkus.silentdisco.core.audio.AudioFileDecoder
import com.ekkus.silentdisco.core.audio.AudioFormatSpec
import com.ekkus.silentdisco.core.audio.DecodedAudioChunk
import com.ekkus.silentdisco.core.audio.ListenerPlaybackScheduler
import com.ekkus.silentdisco.core.audio.AudioTrackPlaybackEngine
import com.ekkus.silentdisco.core.audio.OboeBridge
import com.ekkus.silentdisco.core.audio.PlaybackEngine
import com.ekkus.silentdisco.core.audio.PcmPacketizer
import com.ekkus.silentdisco.core.audio.PlaybackFrame
import com.ekkus.silentdisco.core.audio.PlaybackThresholds
import com.ekkus.silentdisco.core.audio.packetizationStats
import com.ekkus.silentdisco.core.audio.validatePacketBudget
import com.ekkus.silentdisco.core.diagnostics.DiagnosticsStore
import com.ekkus.silentdisco.core.logging.AppLogger
import com.ekkus.silentdisco.core.logging.DiagnosticsMetrics
import com.ekkus.silentdisco.core.model.AppRole
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.JoinApprovalState
import com.ekkus.silentdisco.core.model.JoinRequest
import com.ekkus.silentdisco.core.model.ListenerDiagnosticsSnapshot
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.model.SyncState
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.model.TrustState
import com.ekkus.silentdisco.core.permissions.PermissionCatalogue
import com.ekkus.silentdisco.core.permissions.AppPermission
import com.ekkus.silentdisco.core.permissions.PermissionState
import com.ekkus.silentdisco.core.rust.RustStoredTuningSettings
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.ControlMessage
import com.ekkus.silentdisco.core.protocol.DeviceIdentity
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.protocol.SyncRequestPacket
import com.ekkus.silentdisco.core.protocol.SyncResponsePacket
import com.ekkus.silentdisco.core.sync.HostTimeMapper
import com.ekkus.silentdisco.core.sync.HostTimingService
import com.ekkus.silentdisco.core.sync.ClockSyncEstimator
import com.ekkus.silentdisco.core.sync.ListenerSyncController
import com.ekkus.silentdisco.core.sync.SyncMaintenanceConfig
import com.ekkus.silentdisco.core.transport.BleAdvertisement
import com.ekkus.silentdisco.core.transport.BleDiscoveryService
import com.ekkus.silentdisco.core.transport.BleOperation
import com.ekkus.silentdisco.core.transport.BroadcastDeliverySeverity
import com.ekkus.silentdisco.core.transport.SendAllResult
import com.ekkus.silentdisco.core.transport.classifyBroadcastDelivery
import com.ekkus.silentdisco.core.transport.WifiDirectTransportService
import com.ekkus.silentdisco.platform.persistence.AndroidRustDomainStore
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking

class MainViewModel @JvmOverloads constructor(
    application: Application,
    internal val playbackEngine: PlaybackEngine = AudioTrackPlaybackEngine(),
    internal val domainStore: AndroidRustDomainStore = AndroidRustDomainStore(application),
) : AndroidViewModel(application) {
    internal val logger = AppLogger()
    internal val diagnosticsStore = DiagnosticsStore()
    internal val metrics = DiagnosticsMetrics()
    internal val decoder = AudioFileDecoder(application.contentResolver)
    internal val bleService = BleDiscoveryService(application)
    internal val wifiDirectService = WifiDirectTransportService(application, logger)
    internal val hostTimingService = HostTimingService()

    internal val _uiState = MutableStateFlow(
        AppUiState(
            permissions = PermissionCatalogue.requiredPermissions().map {
                PermissionState(permission = it, granted = false)
            },
            discoveredSessions = emptyList(),
            tuningSettings = TuningSettings(),
        ),
    )
    val uiState: StateFlow<AppUiState> = _uiState.asStateFlow()

    internal var currentSessionId: SessionId? = null
    internal var currentStreamId: StreamId? = null
    internal var listenerSyncController: ListenerSyncController? = null
    internal var listenerScheduler: ListenerPlaybackScheduler? = null
    internal var latestDecodedAudio: AudioDecodeResult? = null
    internal var latestPackets: List<AudioPacket> = emptyList()
    internal val pendingTransportPackets = ArrayDeque<AudioPacket>()
    internal var hostStreamJob: Job? = null
    internal var playbackJob: Job? = null
    internal var resyncJob: Job? = null
    internal var scanJob: Job? = null
    internal var pendingSyncCorrelationId: Long? = null
    internal var pendingJoinRequestMessage: ControlMessage.JoinRequest? = null
    internal val localListenerDeviceId = "listener-device"

    init {
        initializeDomainPersistence()
        observeTransport()
        observeDiscovery()
        observeBleFailures()
    }

    fun selectRole(role: AppRole) {
        _uiState.value = _uiState.value.copy(selectedRole = role)
    }

    fun updatePermission(permission: String, granted: Boolean) {
        _uiState.value = _uiState.value.copy(
            permissions = _uiState.value.permissions.map {
                if (it.permission.androidPermission == permission) it.copy(granted = granted) else it
            },
        )
    }

    fun updateHostForm(
        sessionName: String = _uiState.value.hostForm.sessionName,
        approvalMode: ApprovalMode = _uiState.value.hostForm.approvalMode,
        inviteCode: String = _uiState.value.hostForm.inviteCode,
        rememberApprovedDevices: Boolean = _uiState.value.hostForm.rememberApprovedDevices,
    ) {
        _uiState.value = _uiState.value.copy(
            hostForm = _uiState.value.hostForm.copy(
                sessionName = sessionName,
                approvalMode = approvalMode,
                inviteCode = inviteCode,
                rememberApprovedDevices = rememberApprovedDevices,
            ),
        )
    }

    fun selectAudioFile(uri: Uri, displayName: String, mimeType: String?) {
        runCatching {
            decoder.describe(uri, fallbackName = displayName, fallbackMimeType = mimeType)
        }.onSuccess { selected ->
            latestDecodedAudio = null
            _uiState.value = _uiState.value.copy(
                hostForm = _uiState.value.hostForm.copy(selectedAudio = selected),
                lastMessage = "Selected ${selected.displayName}",
                lastError = null,
            )
        }.onFailure { error ->
            _uiState.value = _uiState.value.copy(lastError = error.message ?: "Failed to access audio file")
        }
    }

    internal fun validateHostForm(state: AppUiState): String? = HostSessionValidator.validate(state.hostForm)

    fun createHostSession(): Boolean = createHostSessionImpl()

    fun addDemoJoinRequest() {
        val sessionId = _uiState.value.hostDiagnostics.sessionId.ifBlank { currentSessionId?.value ?: "demo-session" }
        val request = JoinRequest(
            requestId = UUID.randomUUID().toString(),
            sessionId = sessionId,
            listenerId = UUID.randomUUID().toString(),
            listenerName = "Listener ${_uiState.value.pendingJoinRequests.size + 1}",
            inviteCode = null,
            requestedAtMs = SystemClock.elapsedRealtime(),
        )
        _uiState.value = _uiState.value.copy(
            pendingJoinRequests = _uiState.value.pendingJoinRequests + request,
            hostState = HostLifecycleState.READY,
        )
        refreshHostDiagnostics()
    }

    fun approveJoinRequest(request: JoinRequest) = approveJoinRequestImpl(request)

    fun rejectJoinRequest(request: JoinRequest) = rejectJoinRequestImpl(request)

    fun trustListener(listenerId: String) {
        val displayName = _uiState.value.approvedListeners
            .firstOrNull { it.deviceId == listenerId }
            ?.displayName
            ?: listenerId
        viewModelScope.launch {
            persistTrustedListenerRecord(listenerId, displayName).fold(
                onSuccess = {
                    _uiState.value = _uiState.value.copy(
                        approvedListeners = _uiState.value.approvedListeners.map {
                            if (it.deviceId == listenerId) {
                                it.copy(trustState = TrustState.TRUSTED_PLACEHOLDER)
                            } else {
                                it
                            }
                        },
                        lastMessage = "Trusted listener ${listenerId.take(6)}",
                        lastError = null,
                    )
                    refreshHostDiagnostics()
                },
                onFailure = ::reportTrustedListenerPersistenceFailure,
            )
        }
    }

    internal fun trustedListenerPersistenceMessage(error: Throwable): String =
        "Could not remember listener; approval remains session-only: " +
            (error.message ?: "trusted-device persistence failed")

    fun removeListener(listenerId: String) {
        _uiState.value = _uiState.value.copy(
            approvedListeners = _uiState.value.approvedListeners.filterNot { it.deviceId == listenerId },
            lastMessage = "Removed listener ${listenerId.take(6)}",
        )
        refreshHostDiagnostics()
    }

    fun startHostPlayback() = startHostPlaybackImpl()

    fun pauseHostPlayback() = pauseHostPlaybackImpl()

    fun stopHostPlayback() = stopHostPlaybackImpl()

    fun endSession() = endSessionImpl()

    fun scanForSessions() = scanForSessionsImpl()

    fun selectDiscoveredSession(session: SessionInfo) {
        if (!_uiState.value.canSelectSession(session)) {
            val message = "Finish or cancel the current join before joining another session."
            _uiState.value = _uiState.value.copy(lastError = message)
            diagnosticsStore.updateListener { it.copy(lastError = message) }
            refreshListenerDiagnostics()
            return
        }
        _uiState.value = _uiState.value.copy(
            selectedSession = session,
            listenerState = ListenerLifecycleState.SESSION_SELECTED,
            connectionProgress = _uiState.value.connectionProgress.copy(
                currentState = ListenerLifecycleState.SESSION_SELECTED,
                discovered = true,
                inviteCode = "",
            ),
            lastMessage = "Selected ${session.name}",
            lastError = null,
        )
    }

    fun requestJoin() = requestJoinImpl()

    fun updateInviteCode(code: String) {
        _uiState.value = _uiState.value.copy(
            connectionProgress = _uiState.value.connectionProgress.copy(inviteCode = code),
        )
    }

    fun cancelJoin() {
        clearScanState()
        _uiState.value = _uiState.value.copy(
            listenerState = ListenerLifecycleState.DISCONNECTED,
            connectionProgress = ConnectionProgressState(currentState = ListenerLifecycleState.DISCONNECTED),
            lastError = "Join cancelled",
        )
    }

    fun retryJoin() {
        logger.i("listener.retry", "Retrying listener connection")
        wifiDirectService.retry()
        diagnosticsStore.updateListener {
            it.copy(
                reconnectCount = it.reconnectCount + 1,
                metricsSummary = summarizeMetrics(),
            )
        }
        _uiState.value = _uiState.value.copy(
            listenerState = ListenerLifecycleState.SCANNING,
            connectionProgress = ConnectionProgressState(
                currentState = ListenerLifecycleState.SCANNING,
                discovered = true,
            ),
        )
        scanForSessions()
    }

    fun setLocalVolume(volume: Float) {
        val normalized = volume.coerceIn(0f, 1f)
        playbackEngine.setVolume(normalized)
        _uiState.value = _uiState.value.copy(localVolume = normalized)
    }

    fun adjustTuning(field: TuningField, direction: Int) {
        if (!requirePersistenceReady("update tuning")) return
        val updated = _uiState.value.tuningSettings.adjust(field, direction)
        viewModelScope.launch {
            runCatching {
                domainStore.saveTuning(updated.toRustStoredSettings())
            }.onSuccess {
                listenerSyncController = _uiState.value.selectedSession
                    ?.let { createSyncController(SessionId(it.id)) }
                _uiState.value = _uiState.value.copy(
                    tuningSettings = updated,
                    lastMessage = "Updated tuning: ${updated.summary()}",
                    lastError = null,
                )
                if (resyncJob?.isActive == true) {
                    startPeriodicListenerResync()
                }
            }.onFailure { error ->
                val message = error.message ?: "Failed to persist tuning settings"
                logger.e("storage.settings", message, error)
                _uiState.value = _uiState.value.copy(lastError = message)
            }
        }
    }

    fun leaveSession() {
        clearScanState()
        hostStreamJob?.cancel()
        playbackJob?.cancel()
        resyncJob?.cancel()
        listenerScheduler = null
        pendingTransportPackets.clear()
        pendingSyncCorrelationId = null
        pendingJoinRequestMessage = null
        logger.i("listener.disconnect", "Listener left session")
        _uiState.value = _uiState.value.copy(
            listenerState = ListenerLifecycleState.IDLE,
            listenerPlaybackState = PlaybackState.STOPPED,
            selectedSession = null,
            connectionProgress = ConnectionProgressState(),
        )
        diagnosticsStore.updateListener { ListenerDiagnosticsSnapshot() }
        refreshListenerDiagnostics()
    }

    fun manualResync() {
        if (!_uiState.value.canManualResync()) {
            val message = "Join a session before requesting manual resync"
            _uiState.value = _uiState.value.copy(lastError = message)
            diagnosticsStore.updateListener { it.copy(lastError = message) }
            refreshListenerDiagnostics()
            return
        }
        requestListenerSyncProbe(source = "Manual resync")
    }

    internal fun generateInviteCode(): String =
        ((SystemClock.elapsedRealtime() % 9_000) + 1_000).toString()

    internal fun currentPlaybackThresholds(): PlaybackThresholds = PlaybackThresholds(
        startupBufferMs = _uiState.value.tuningSettings.startupBufferMs,
        softCorrectionThresholdMs = _uiState.value.tuningSettings.latePacketThresholdMs,
        hardResyncThresholdMs = _uiState.value.tuningSettings.hardResyncThresholdMs,
    )

    internal fun shouldKeepResyncing(): Boolean =
        _uiState.value.listenerState in setOf(
            ListenerLifecycleState.APPROVED,
            ListenerLifecycleState.CONNECTING,
            ListenerLifecycleState.SYNCING_CLOCK,
            ListenerLifecycleState.BUFFERING,
            ListenerLifecycleState.PLAYING,
            ListenerLifecycleState.RECONNECTING,
            ListenerLifecycleState.DESYNCED,
        )

    fun retryDomainPersistence() {
        if (!_uiState.value.canRetryStorageInitialization()) return
        initializeDomainPersistence()
    }

    internal fun hasPermission(permission: AppPermission): Boolean =
        _uiState.value.permissions.any { it.permission == permission && it.granted }

    internal fun hasHostTransportPermissions(): Boolean =
        PermissionCatalogue.wifiDirectPermissions().all { hasPermission(it) } &&
            PermissionCatalogue.bluetoothPermissions().all { hasPermission(it) }

    internal fun hasListenerTransportPermissions(): Boolean =
        PermissionCatalogue.wifiDirectPermissions().all { hasPermission(it) } &&
            PermissionCatalogue.bluetoothPermissions().all { hasPermission(it) }

    internal fun demoSessions(): List<SessionInfo> = listOf(
        SessionInfo(
            id = "demo-session-alpha",
            name = "Back Patio Demo",
            hostDeviceName = "Pixel Host",
            approvalMode = ApprovalMode.MANUAL,
            inviteCodeRequired = false,
        ),
        SessionInfo(
            id = "demo-session-beta",
            name = "Invite Code Demo",
            hostDeviceName = "Galaxy Host",
            approvalMode = ApprovalMode.INVITE_CODE,
            inviteCodeRequired = true,
        ),
    )

    override fun onCleared() {
        bleService.stop()
        wifiDirectService.stop()
        runBlocking(Dispatchers.IO) { domainStore.close() }
        super.onCleared()
    }
}

internal fun RustStoredTuningSettings.toAppTuningSettings(): TuningSettings = TuningSettings(
    syncSampleWindow = syncSampleWindow,
    syncCadenceMs = syncCadenceMs,
    startupBufferMs = startupBufferMs,
    latePacketThresholdMs = latePacketThresholdMs,
    hardResyncThresholdMs = hardResyncThresholdMs,
    syncDriftThresholdMs = syncDriftThresholdMs,
    scanWindowMs = scanWindowMs,
)

internal fun TuningSettings.toRustStoredSettings(): RustStoredTuningSettings =
    RustStoredTuningSettings(
        syncSampleWindow = syncSampleWindow,
        syncCadenceMs = syncCadenceMs,
        startupBufferMs = startupBufferMs,
        latePacketThresholdMs = latePacketThresholdMs,
        hardResyncThresholdMs = hardResyncThresholdMs,
        syncDriftThresholdMs = syncDriftThresholdMs,
        scanWindowMs = scanWindowMs,
        updatedAtMs = System.currentTimeMillis(),
    )

internal fun requireHostSessionForPlayback(currentSessionId: SessionId?): String? =
    if (currentSessionId == null) "Start a host session before starting playback" else null

internal enum class TransportSnapshotRole { HOST_FAILURE, LISTENER_FAILURE, IGNORE }

internal fun classifyTransportSnapshotRole(
    hostState: HostLifecycleState,
    listenerState: ListenerLifecycleState,
): TransportSnapshotRole {
    val hosting = hostState in setOf(
        HostLifecycleState.CREATING_SESSION,
        HostLifecycleState.WAITING_FOR_LISTENERS,
        HostLifecycleState.READY,
        HostLifecycleState.STREAMING,
    )
    if (hosting) return TransportSnapshotRole.HOST_FAILURE
    val listenerActiveOrJoining = listenerState in setOf(
        ListenerLifecycleState.SCANNING,
        ListenerLifecycleState.SESSION_SELECTED,
        ListenerLifecycleState.JOIN_REQUESTED,
        ListenerLifecycleState.CONNECTING,
        ListenerLifecycleState.AWAITING_APPROVAL,
        ListenerLifecycleState.APPROVED,
        ListenerLifecycleState.SYNCING_CLOCK,
        ListenerLifecycleState.BUFFERING,
        ListenerLifecycleState.PLAYING,
        ListenerLifecycleState.RECONNECTING,
        ListenerLifecycleState.DESYNCED,
    )
    if (listenerActiveOrJoining) return TransportSnapshotRole.LISTENER_FAILURE
    return TransportSnapshotRole.IGNORE
}

internal fun nextStateForSyncProbe(currentState: ListenerLifecycleState): ListenerLifecycleState =
    if (currentState in setOf(
            ListenerLifecycleState.APPROVED,
            ListenerLifecycleState.CONNECTING,
            ListenerLifecycleState.SYNCING_CLOCK,
        )
    ) {
        ListenerLifecycleState.SYNCING_CLOCK
    } else {
        currentState
    }
