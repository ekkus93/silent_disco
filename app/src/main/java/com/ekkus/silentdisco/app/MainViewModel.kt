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
import com.ekkus.silentdisco.core.audio.OboeBridge
import com.ekkus.silentdisco.core.audio.OboePlaybackEngine
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
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import com.ekkus.silentdisco.core.rust.HostCoreController
import com.ekkus.silentdisco.core.rust.HostTransportController
import com.ekkus.silentdisco.core.rust.ListenerCoreController
import com.ekkus.silentdisco.core.rust.ListenerTransportController
import com.ekkus.silentdisco.core.rust.ManualListenerTransportController
import com.ekkus.silentdisco.core.rust.RustStoredTuningSettings
import com.ekkus.silentdisco.core.rust.UniFfiHostCoreController
import com.ekkus.silentdisco.core.rust.UniFfiListenerCoreController
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
    internal val playbackEngine: PlaybackEngine = OboePlaybackEngine(),
    internal val domainStore: AndroidRustDomainStore = AndroidRustDomainStore(application),
    internal val hostCoreFactory: (String) -> HostCoreController = {
        UniFfiHostCoreController(it)
    },
    internal val listenerCoreFactory: (String) -> ListenerCoreController = {
        UniFfiListenerCoreController(it)
    },
) : AndroidViewModel(application) {
    internal val logger = AppLogger()
    internal val diagnosticsStore = DiagnosticsStore()
    internal val metrics = DiagnosticsMetrics()
    internal val decoder = AudioFileDecoder(application.contentResolver)
    internal val bleService = BleDiscoveryService(application)
    internal val wifiDirectService = WifiDirectTransportService(application, logger)
    internal val hostTimingService = HostTimingService()
    internal val manualListenerController = ManualListenerTransportController(
        application.getExternalFilesDir(null),
        networkSessionLock = WifiLowLatencyNetworkLock(application),
    )
    internal val hostTransportController = HostTransportController()
    internal val listenerTransportController = ListenerTransportController()

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
    internal var listenerPlayback: FfiListenerPlaybackHandle? = null
    internal var latestDecodedAudio: AudioDecodeResult? = null
    internal var latestPackets: List<AudioPacket> = emptyList()
    internal val pendingTransportPackets = ArrayDeque<AudioPacket>()
    internal var hostStreamJob: Job? = null
    internal var hostPlaybackCommandJob: Job? = null
    internal var playbackJob: Job? = null
    internal var resyncJob: Job? = null
    internal var scanJob: Job? = null
    internal var pendingSyncCorrelationId: Long? = null
    internal var hostCoreController: HostCoreController? = null
    internal var listenerCoreController: ListenerCoreController? = null
    internal var pendingEstablishNetworkOperationId: String? = null
    internal var pendingStartAdvertisingOperationId: String? = null
    internal var hostTransportEventLoopStarted: Boolean = false
    internal var listenerTransportEventLoopStarted: Boolean = false
    internal val localListenerDeviceId = "listener-device"

    /**
     * A listener's sync/audio UDP ports, from its `JoinRequestReceived`
     * event, kept only until the join is approved or otherwise resolved --
     * `authorize_listener` needs them, but nothing else does, so they never
     * belong in Rust-owned join-request state.
     */
    internal val pendingListenerDatagramPorts = mutableMapOf<String, Pair<UShort, UShort>>()

    init {
        initializeDomainPersistence()
        observeDiscovery()
        observeBleFailures()
        observeManualEndpointConnection()
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

    fun createHostSession() = createRustHostSession()

    fun addDemoJoinRequest() = submitDemoRustJoinRequest()

    fun approveJoinRequest(
        request: JoinRequest,
        rememberForFuture: Boolean = _uiState.value.hostForm.rememberApprovedDevices,
    ) = approveRustJoinRequest(request, rememberForFuture)

    fun rejectJoinRequest(request: JoinRequest) = rejectRustJoinRequest(request)

    fun removeListener(listenerId: String) = removeRustListener(listenerId)

    fun startHostPlayback() = startHostPlaybackImpl()

    fun pauseHostPlayback() = pauseHostPlaybackImpl()

    fun stopHostPlayback() = stopHostPlaybackImpl()

    fun endSession() = endRustHostSession()

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
            connectionProgress = _uiState.value.connectionProgress.copy(inviteCode = ""),
            lastMessage = "Selected ${session.name}",
            lastError = null,
        )
        if (session.id.startsWith(DEMO_SESSION_ID_PREFIX)) {
            // Demo sessions are a BuildConfig.DEBUG-only simulation that never
            // touches Rust (see requestJoinImpl's demo branch); Rust doesn't
            // know about them, so select_session would just reject them.
            _uiState.value = _uiState.value.copy(
                selectedSession = session,
                listenerState = ListenerLifecycleState.SESSION_SELECTED,
                connectionProgress = _uiState.value.connectionProgress.copy(
                    currentState = ListenerLifecycleState.SESSION_SELECTED,
                    discovered = true,
                ),
            )
            return
        }
        ensureRustListenerCore().selectSession(session.id)
    }

    fun requestJoin() = requestJoinImpl()

    fun updateManualEndpointInput(raw: String) = updateManualEndpointInputImpl(raw)

    fun updateManualEndpointInviteCode(code: String) = updateManualEndpointInviteCodeImpl(code)

    fun connectManualEndpoint() = connectManualEndpointImpl()

    fun cancelManualEndpointConnect() = cancelManualEndpointConnectImpl()

    fun updateInviteCode(code: String) {
        _uiState.value = _uiState.value.copy(
            connectionProgress = _uiState.value.connectionProgress.copy(inviteCode = code),
        )
    }

    fun cancelJoin() {
        clearScanState()
        pendingEstablishNetworkOperationId = null
        _uiState.value = _uiState.value.copy(lastError = "Join cancelled")
        listenerCoreController?.cancelJoin()
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
        listenerCoreController?.retryRecoverableFailure()
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
        stopListenerPlayback()
        resyncJob?.cancel()
        pendingSyncCorrelationId = null
        pendingEstablishNetworkOperationId = null
        logger.i("listener.disconnect", "Listener left session")
        _uiState.value = _uiState.value.copy(listenerPlaybackState = PlaybackState.STOPPED)
        listenerCoreController?.cancelJoin()
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
            PermissionCatalogue.bluetoothPermissions()
                // Listeners scan and connect but never advertise -- matches
                // PermissionRequestContext.LISTENER_NEARBY, which never
                // requests BluetoothAdvertise, so it would otherwise never
                // be granted through the listener's own request flow.
                .filter { it != AppPermission.BluetoothAdvertise }
                .all { hasPermission(it) }

    internal fun demoSessions(): List<SessionInfo> = listOf(
        SessionInfo(
            id = "${DEMO_SESSION_ID_PREFIX}alpha",
            name = "Back Patio Demo",
            hostDeviceName = "Pixel Host",
            approvalMode = ApprovalMode.MANUAL,
            inviteCodeRequired = false,
        ),
        SessionInfo(
            id = "${DEMO_SESSION_ID_PREFIX}beta",
            name = "Invite Code Demo",
            hostDeviceName = "Galaxy Host",
            approvalMode = ApprovalMode.INVITE_CODE,
            inviteCodeRequired = true,
        ),
    )

    override fun onCleared() {
        hostPlaybackCommandJob?.cancel()
        bleService.stop()
        wifiDirectService.stop()
        manualListenerController.close()
        hostCoreController?.close()
        listenerCoreController?.close()
        hostTransportController.close()
        listenerTransportController.close()
        runBlocking(Dispatchers.IO) { domainStore.close() }
        super.onCleared()
    }
}

/** Prefix for [MainViewModel.demoSessions], a BuildConfig.DEBUG-only simulation that never touches Rust. */
internal const val DEMO_SESSION_ID_PREFIX = "demo-session-"

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
