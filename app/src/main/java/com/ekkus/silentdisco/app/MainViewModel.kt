package com.ekkus.silentdisco.app

import android.app.Application
import android.content.Context
import android.net.Uri
import android.os.SystemClock
import androidx.core.content.edit
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.core.audio.AudioDecodeResult
import com.ekkus.silentdisco.core.audio.AudioFileAccessException
import com.ekkus.silentdisco.core.audio.AudioFileDecoder
import com.ekkus.silentdisco.core.audio.AudioFormatSpec
import com.ekkus.silentdisco.core.audio.DecodedAudioChunk
import com.ekkus.silentdisco.core.audio.ListenerPlaybackScheduler
import com.ekkus.silentdisco.core.audio.OboeBridge
import com.ekkus.silentdisco.core.audio.OboePlaybackEngine
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
import com.ekkus.silentdisco.core.transport.WifiDirectTransportService
import java.util.UUID
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

class MainViewModel(application: Application) : AndroidViewModel(application) {
    private val logger = AppLogger()
    private val diagnosticsStore = DiagnosticsStore()
    private val metrics = DiagnosticsMetrics()
    private val decoder = AudioFileDecoder(application.contentResolver)
    private val bleService = BleDiscoveryService(application)
    private val wifiDirectService = WifiDirectTransportService(application, logger)
    private val hostTimingService = HostTimingService()
    private val playbackEngine = OboePlaybackEngine()
    private val preferences = application.getSharedPreferences("silent-disco", Context.MODE_PRIVATE)

    private val _uiState = MutableStateFlow(
        AppUiState(
            permissions = PermissionCatalogue.requiredPermissions().map {
                PermissionState(permission = it, granted = false)
            },
            discoveredSessions = emptyList(),
            tuningSettings = loadTuningSettings(),
        ),
    )
    val uiState: StateFlow<AppUiState> = _uiState.asStateFlow()

    private var currentSessionId: SessionId? = null
    private var currentStreamId: StreamId? = null
    private var listenerSyncController: ListenerSyncController? = null
    private var listenerScheduler: ListenerPlaybackScheduler? = null
    private var latestDecodedAudio: AudioDecodeResult? = null
    private var latestPackets: List<AudioPacket> = emptyList()
    private val pendingTransportPackets = ArrayDeque<AudioPacket>()
    private var hostStreamJob: Job? = null
    private var playbackJob: Job? = null
    private var resyncJob: Job? = null
    private var pendingSyncCorrelationId: Long? = null
    private var pendingJoinRequestMessage: ControlMessage.JoinRequest? = null
    private val localListenerDeviceId = "listener-device"

    init {
        observeTransport()
        observeDiscovery()
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
        val resolvedInviteCode = if (approvalMode == ApprovalMode.INVITE_CODE && inviteCode.isBlank()) {
            generateInviteCode()
        } else {
            inviteCode
        }
        _uiState.value = _uiState.value.copy(
            hostForm = _uiState.value.hostForm.copy(
                sessionName = sessionName,
                approvalMode = approvalMode,
                inviteCode = resolvedInviteCode,
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

    fun createHostSession(): Boolean {
        val form = _uiState.value.hostForm
        if (form.sessionName.isBlank()) {
            _uiState.value = _uiState.value.copy(lastError = "Session name is required")
            return false
        }
        if (!hasHostTransportPermissions()) {
            val message = "Missing nearby connectivity permissions for advertising"
            wifiDirectService.fail(message, retryable = true)
            _uiState.value = _uiState.value.copy(
                hostState = HostLifecycleState.ERROR,
                lastError = message,
            )
            refreshHostDiagnostics(streamState = PlaybackState.ERROR)
            return false
        }
        currentSessionId = SessionId(UUID.randomUUID().toString())
        currentStreamId = StreamId("stream-${SystemClock.elapsedRealtime()}")
        val session = SessionInfo(
            id = currentSessionId!!.value,
            name = form.sessionName.trim(),
            hostDeviceName = "This Android Host",
            approvalMode = form.approvalMode,
            inviteCodeRequired = form.approvalMode == ApprovalMode.INVITE_CODE,
        )
        return runCatching {
            bleService.startAdvertising(
                BleAdvertisement(
                    sessionId = session.id,
                    sessionName = session.name,
                    hostName = session.hostDeviceName,
                    approvalRequired = true,
                    inviteCodeRequired = session.inviteCodeRequired,
                ),
            )
            wifiDirectService.startHost(session)
        }.onSuccess {
            logger.i("transport.host", "Started BLE discovery bridge for ${session.id}")
            diagnosticsStore.updateHost {
                it.copy(
                    sessionId = session.id,
                    streamState = PlaybackState.STOPPED,
                    lastContactElapsedMs = SystemClock.elapsedRealtime(),
                    lastError = null,
                )
            }
            _uiState.value = _uiState.value.copy(
                hostState = HostLifecycleState.WAITING_FOR_LISTENERS,
                discoveredSessions = listOf(session),
                lastMessage = "Hosting ${session.name}",
                lastError = null,
            )
            refreshHostDiagnostics()
        }.onFailure { error ->
            logger.e("transport.host", "Failed to create host session", error)
            wifiDirectService.fail("Failed to start host session", retryable = true)
            _uiState.value = _uiState.value.copy(
                hostState = HostLifecycleState.ERROR,
                lastError = error.message ?: "Failed to create host session",
            )
            refreshHostDiagnostics(streamState = PlaybackState.ERROR, sessionId = session.id)
        }.isSuccess
    }

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

    fun approveJoinRequest(request: JoinRequest) {
        if (_uiState.value.hostForm.rememberApprovedDevices) {
            trustListener(request.listenerId)
        }
        logger.i("approval.approve", "Approved ${request.listenerName}")
        _uiState.value = _uiState.value.copy(
            pendingJoinRequests = _uiState.value.pendingJoinRequests - request,
            approvedListeners = (_uiState.value.approvedListeners + request.toListenerInfo()).distinctBy { it.deviceId },
            lastMessage = "${request.listenerName} approved",
            lastError = null,
        )
        viewModelScope.launch {
            runCatching {
                wifiDirectService.broadcastControl(
                    ControlMessage.JoinApproval(
                        version = 1,
                        sessionId = SessionId(request.sessionId),
                        listenerId = request.listenerId,
                        trustedForFuture = _uiState.value.hostForm.rememberApprovedDevices,
                    ),
                )
            }.onFailure { error ->
                handleListenerConnectionFailure(error.message ?: "Failed to send join approval")
            }
        }
        refreshHostDiagnostics()
    }

    fun rejectJoinRequest(request: JoinRequest) {
        logger.w("approval.reject", "Rejected ${request.listenerName}")
        _uiState.value = _uiState.value.copy(
            pendingJoinRequests = _uiState.value.pendingJoinRequests - request,
            lastMessage = "${request.listenerName} rejected",
            lastError = "Host rejected ${request.listenerName}",
        )
        diagnosticsStore.updateListener {
            it.copy(
                sessionId = request.sessionId,
                lastError = "Host rejected join request for ${request.listenerName}",
            )
        }
        viewModelScope.launch {
            runCatching {
                wifiDirectService.broadcastControl(
                    ControlMessage.JoinRejection(
                        version = 1,
                        sessionId = SessionId(request.sessionId),
                        listenerId = request.listenerId,
                        reason = "Host rejected ${request.listenerName}",
                    ),
                )
            }
        }
        refreshHostDiagnostics()
        refreshListenerDiagnostics()
    }

    fun trustListener(listenerId: String) {
        preferences.edit { putBoolean("trusted:$listenerId", true) }
        _uiState.value = _uiState.value.copy(
            approvedListeners = _uiState.value.approvedListeners.map {
                if (it.deviceId == listenerId) it.copy(trustState = TrustState.TRUSTED_PLACEHOLDER) else it
            },
            lastMessage = "Trusted listener ${listenerId.take(6)}",
        )
        refreshHostDiagnostics()
    }

    fun removeListener(listenerId: String) {
        _uiState.value = _uiState.value.copy(
            approvedListeners = _uiState.value.approvedListeners.filterNot { it.deviceId == listenerId },
            lastMessage = "Removed listener ${listenerId.take(6)}",
        )
        refreshHostDiagnostics()
    }

    fun startHostPlayback() {
        val selectedAudio = _uiState.value.hostForm.selectedAudio
        if (selectedAudio == null) {
            _uiState.value = _uiState.value.copy(lastError = "Choose an audio file before starting playback")
            return
        }
        val sessionId = currentSessionId ?: SessionId(_uiState.value.hostDiagnostics.sessionId.ifBlank { UUID.randomUUID().toString() })
        val streamId = currentStreamId ?: StreamId("stream-${SystemClock.elapsedRealtime()}")
        runCatching {
            latestDecodedAudio ?: decoder.decode(selectedAudio)
        }.onSuccess { decoded ->
            latestDecodedAudio = decoded
            val combinedBytes = decoded.chunks.fold(ByteArray(0)) { acc, chunk -> acc + chunk.pcm16Le }
            val packetizer = PcmPacketizer(
                sessionId = sessionId,
                streamId = streamId,
                format = decoded.format,
            )
            latestPackets = packetizer.packetize(
                chunk = DecodedAudioChunk(
                    pcm16Le = combinedBytes,
                    firstSampleIndex = 0,
                    frameCount = combinedBytes.size / decoded.format.bytesPerFrame,
                ),
                hostPresentationStartMs = SystemClock.elapsedRealtime() + 500,
            )
            if (latestPackets.isEmpty()) {
                error("Decoded stream produced no playable packets")
            }
            val packetBudget = latestPackets.validatePacketBudget()
            val packetStats = latestPackets.packetizationStats()
            if (!packetBudget.valid) {
                error("Packet budget exceeded: ${packetBudget.maxPacketBytes} bytes")
            }
            metrics.increment("stream_start")
            metrics.recordTiming("packet_duration_ms", 20.0)
            metrics.recordTiming("average_packet_bytes", packetStats.averagePacketBytes)
            diagnosticsStore.updateHost {
                it.copy(
                    packetSendCount = 0,
                    packetSendRatePerSecond = 0.0,
                    packetBudgetSummary = packetBudget.summary(),
                    streamState = PlaybackState.PLAYING,
                    lastContactElapsedMs = SystemClock.elapsedRealtime(),
                    metricsSummary = summarizeMetrics(),
                    lastError = null,
                )
            }
            val backend = playbackEngine.start(decoded.format)
            logger.i(
                "stream.start",
                "stream=${streamId.value} packets=${latestPackets.size} budget=${packetBudget.summary()}",
            )
            _uiState.value = _uiState.value.copy(
                hostState = HostLifecycleState.STREAMING,
                hostPlaybackState = PlaybackState.PLAYING,
                lastMessage = "Host stream started via $backend",
                lastError = null,
            )
            viewModelScope.launch {
                runCatching {
                    wifiDirectService.broadcastControl(
                        ControlMessage.StreamStart(
                            version = 1,
                            sessionId = sessionId,
                            streamId = streamId,
                            hostStartTimeMs = latestPackets.first().hostPresentationTimeMs,
                            sampleRate = decoded.format.sampleRate,
                            channels = decoded.format.channelCount,
                            samplesPerPacket = latestPackets.first().samplesPerPacket,
                        ),
                    )
                }.onFailure { error ->
                    logger.w("transport.control", "Failed to send stream start: ${error.message}")
                }
            }
            refreshHostDiagnostics()
            startHostStreamingLoop(streamId)
            startPeriodicResync()
        }.onFailure { error ->
            metrics.increment("stream_start_error")
            logger.e("stream.start", "Failed to start host playback", error)
            _uiState.value = _uiState.value.copy(
                hostState = HostLifecycleState.ERROR,
                hostPlaybackState = PlaybackState.ERROR,
                lastError = error.message ?: "Failed to decode audio file",
            )
            diagnosticsStore.updateHost { it.copy(lastError = error.message, streamState = PlaybackState.ERROR) }
            refreshHostDiagnostics(streamState = PlaybackState.ERROR)
        }
    }

    fun pauseHostPlayback() {
        logger.i("stream.pause", "Pausing host stream")
        _uiState.value = _uiState.value.copy(
            hostState = HostLifecycleState.PAUSED,
            hostPlaybackState = PlaybackState.PAUSED,
        )
        metrics.increment("stream_pause")
        propagateListenerPlaybackState(
            playbackState = PlaybackState.PAUSED,
            listenerState = _uiState.value.listenerState,
            message = "Host paused the stream",
        )
        currentSessionId?.let { sessionId ->
            currentStreamId?.let { streamId ->
                viewModelScope.launch {
                    runCatching {
                        wifiDirectService.broadcastControl(
                            ControlMessage.Pause(
                                version = 1,
                                sessionId = sessionId,
                                streamId = streamId,
                                hostPauseTimeMs = SystemClock.elapsedRealtime(),
                            ),
                        )
                    }
                }
            }
        }
        refreshHostDiagnostics(streamState = PlaybackState.PAUSED)
    }

    fun stopHostPlayback() {
        logger.i("stream.stop", "Stopping host stream")
        hostStreamJob?.cancel()
        playbackJob?.cancel()
        resyncJob?.cancel()
        playbackEngine.stop()
        _uiState.value = _uiState.value.copy(
            hostState = HostLifecycleState.READY,
            hostPlaybackState = PlaybackState.STOPPED,
        )
        metrics.increment("stream_stop")
        propagateListenerPlaybackState(
            playbackState = PlaybackState.STOPPED,
            listenerState = _uiState.value.listenerState,
            message = "Host stopped the stream",
        )
        currentSessionId?.let { sessionId ->
            currentStreamId?.let { streamId ->
                viewModelScope.launch {
                    runCatching {
                        wifiDirectService.broadcastControl(
                            ControlMessage.Stop(
                                version = 1,
                                sessionId = sessionId,
                                streamId = streamId,
                                hostStopTimeMs = SystemClock.elapsedRealtime(),
                            ),
                        )
                    }
                }
            }
        }
        refreshHostDiagnostics(streamState = PlaybackState.STOPPED)
    }

    fun endSession() {
        currentSessionId?.let { sessionId ->
            viewModelScope.launch {
                runCatching {
                    wifiDirectService.broadcastControl(
                        ControlMessage.Disconnect(
                            version = 1,
                            sessionId = sessionId,
                            listenerId = localListenerDeviceId,
                            reason = "Host ended the session",
                        ),
                    )
                }
            }
        }
        stopHostPlayback()
        bleService.stop()
        wifiDirectService.stop()
        logger.i("session.end", "Session ended by host")
        _uiState.value = _uiState.value.copy(
            hostState = HostLifecycleState.IDLE,
            pendingJoinRequests = emptyList(),
            approvedListeners = emptyList(),
            hostPlaybackState = PlaybackState.STOPPED,
            lastMessage = "Session ended",
        )
        if (_uiState.value.selectedSession != null) {
            _uiState.value = _uiState.value.copy(
                listenerState = ListenerLifecycleState.DISCONNECTED,
                listenerPlaybackState = PlaybackState.STOPPED,
                lastError = "Host ended the session",
            )
        }
        refreshHostDiagnostics(streamState = PlaybackState.STOPPED, sessionId = "")
    }

    fun scanForSessions() {
        logger.i("listener.scan", "Scanning for nearby sessions")
        if (!hasListenerTransportPermissions()) {
            val message = "Missing nearby connectivity permissions for discovery"
            wifiDirectService.fail(message, retryable = true)
            _uiState.value = _uiState.value.copy(
                listenerState = ListenerLifecycleState.ERROR,
                lastError = message,
            )
            diagnosticsStore.updateListener { it.copy(lastError = message) }
            refreshListenerDiagnostics()
            return
        }
        bleService.startScanning()
        wifiDirectService.discoverPeers()
        refreshDiscoveredSessions()
        val discovered = _uiState.value.discoveredSessions
        _uiState.value = _uiState.value.copy(
            listenerState = ListenerLifecycleState.SCANNING,
            discoveredSessions = discovered,
            connectionProgress = _uiState.value.connectionProgress.copy(
                currentState = ListenerLifecycleState.SCANNING,
                discovered = discovered.isNotEmpty(),
            ),
            lastError = null,
        )
        diagnosticsStore.updateListener {
            it.copy(lastError = null)
        }
        refreshListenerDiagnostics()
    }

    fun selectDiscoveredSession(session: SessionInfo) {
        _uiState.value = _uiState.value.copy(
            selectedSession = session,
            listenerState = ListenerLifecycleState.SESSION_SELECTED,
            connectionProgress = _uiState.value.connectionProgress.copy(
                currentState = ListenerLifecycleState.SESSION_SELECTED,
                discovered = true,
            ),
        )
    }

    fun requestJoin() {
        val session = _uiState.value.selectedSession ?: run {
            _uiState.value = _uiState.value.copy(lastError = "Select a session before joining")
            return
        }
        if (_uiState.value.discoveredSessions.none { it.id == session.id }) {
            _uiState.value = _uiState.value.copy(lastError = "Selected session disappeared before join")
            diagnosticsStore.updateListener { it.copy(lastError = "Selected session disappeared before join") }
            refreshListenerDiagnostics()
            return
        }
        if (session.inviteCodeRequired && _uiState.value.connectionProgress.inviteCode.isBlank()) {
            _uiState.value = _uiState.value.copy(lastError = "Invite code required")
            return
        }
        val request = ControlMessage.JoinRequest(
            version = 1,
            sessionId = SessionId(session.id),
            device = DeviceIdentity(localListenerDeviceId, "This Android Listener"),
            inviteCode = _uiState.value.connectionProgress.inviteCode.ifBlank { null },
        )
        logger.i("listener.join", "Join request created for ${request.sessionId.value}")
        _uiState.value = _uiState.value.copy(
            listenerState = ListenerLifecycleState.CONNECTING,
            connectionProgress = _uiState.value.connectionProgress.copy(
                currentState = ListenerLifecycleState.CONNECTING,
                requested = true,
            ),
            lastMessage = "Connecting to host",
            lastError = null,
        )
        pendingJoinRequestMessage = request
        val shouldSimulate = session.id.startsWith("demo-session-")
        if (shouldSimulate) {
            val shouldReject = session.inviteCodeRequired && request.inviteCode != "1234"
            simulateApprovalAndPlayback(session.id, shouldReject)
            return
        }
        wifiDirectService.connectToSession(session)
    }

    fun updateInviteCode(code: String) {
        _uiState.value = _uiState.value.copy(
            connectionProgress = _uiState.value.connectionProgress.copy(inviteCode = code),
        )
    }

    fun cancelJoin() {
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
        _uiState.value = _uiState.value.copy(localVolume = volume)
    }

    fun adjustTuning(field: TuningField, direction: Int) {
        val updated = _uiState.value.tuningSettings.adjust(field, direction)
        persistTuningSettings(updated)
        listenerSyncController = _uiState.value.selectedSession?.let { createSyncController(SessionId(it.id)) }
        _uiState.value = _uiState.value.copy(
            tuningSettings = updated,
            lastMessage = "Updated tuning: ${updated.summary()}",
            lastError = null,
        )
        if (resyncJob?.isActive == true) {
            startPeriodicResync()
        }
    }

    fun leaveSession() {
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
        val controller = listenerSyncController ?: _uiState.value.selectedSession?.let {
            createSyncController(SessionId(it.id))
        } ?: return
        listenerSyncController = controller
        val request = controller.newProbe()
        pendingSyncCorrelationId = request.correlationId
        if (wifiDirectService.snapshot.value.state == TransportConnectionState.CONNECTED) {
            viewModelScope.launch {
                runCatching {
                    wifiDirectService.sendSyncRequestToHost(request)
                }.onFailure { error ->
                    handleSyncFailure(error.message ?: "Failed to send sync probe")
                }
            }
            return
        }
        applySyncResponse(hostTimingService.createResponse(request))
    }

    private fun simulateApprovalAndPlayback(sessionId: String, reject: Boolean) {
        viewModelScope.launch {
            delay(500)
            if (reject) {
                _uiState.value = _uiState.value.copy(
                    listenerState = ListenerLifecycleState.ERROR,
                    lastError = "Host rejected join. Check the invite code and try again.",
                )
                diagnosticsStore.updateListener {
                    it.copy(sessionId = sessionId, lastError = "Join rejected by host")
                }
                refreshListenerDiagnostics()
                return@launch
            }
            _uiState.value = _uiState.value.copy(
                listenerState = ListenerLifecycleState.APPROVED,
                connectionProgress = _uiState.value.connectionProgress.copy(
                    currentState = ListenerLifecycleState.APPROVED,
                    approved = true,
                ),
            )
            delay(400)
            wifiDirectService.connectToSession(_uiState.value.selectedSession ?: demoSessions().first())
            if (wifiDirectService.snapshot.value.state != TransportConnectionState.CONNECTED) {
                handleListenerConnectionFailure("Failed to connect to host transport")
                return@launch
            }
            _uiState.value = _uiState.value.copy(
                listenerState = ListenerLifecycleState.CONNECTING,
                connectionProgress = _uiState.value.connectionProgress.copy(
                    currentState = ListenerLifecycleState.CONNECTING,
                    connected = true,
                ),
            )
            delay(400)
            manualResync()
            _uiState.value = _uiState.value.copy(
                listenerState = ListenerLifecycleState.BUFFERING,
                connectionProgress = _uiState.value.connectionProgress.copy(
                    currentState = ListenerLifecycleState.BUFFERING,
                    synced = true,
                ),
            )
            startListenerPlaybackSimulation(sessionId)
        }
    }

    private fun observeTransport() {
        viewModelScope.launch {
            wifiDirectService.controlMessages.collect(::handleControlMessage)
        }
        viewModelScope.launch {
            wifiDirectService.syncRequests.collect { request ->
                if (request.sessionId != currentSessionId) return@collect
                runCatching {
                    wifiDirectService.broadcastSyncResponse(hostTimingService.createResponse(request))
                }.onFailure { error ->
                    logger.w("transport.sync", "Failed to answer sync probe: ${error.message}")
                }
            }
        }
        viewModelScope.launch {
            wifiDirectService.syncResponses.collect(::applySyncResponse)
        }
        viewModelScope.launch {
            wifiDirectService.audioPackets.collect(::handleIncomingAudioPacket)
        }
    }

    private fun observeDiscovery() {
        viewModelScope.launch {
            bleService.discoveredSessions.collect {
                refreshDiscoveredSessions()
            }
        }
        viewModelScope.launch {
            wifiDirectService.snapshot.collect { snapshot ->
                refreshDiscoveredSessions()
                if (pendingJoinRequestMessage != null && snapshot.state == TransportConnectionState.CONNECTED) {
                    sendPendingJoinRequest()
                }
                if (_uiState.value.listenerState == ListenerLifecycleState.SCANNING &&
                    _uiState.value.discoveredSessions.isEmpty() &&
                    snapshot.state == TransportConnectionState.DISCOVERING
                ) {
                    _uiState.value = _uiState.value.copy(lastMessage = "Scanning for nearby sessions...")
                }
                snapshot.lastError?.let { error ->
                    if (_uiState.value.listenerState == ListenerLifecycleState.CONNECTING ||
                        _uiState.value.listenerState == ListenerLifecycleState.SCANNING
                    ) {
                        _uiState.value = _uiState.value.copy(lastError = error.message)
                    }
                }
            }
        }
    }

    private fun handleControlMessage(message: ControlMessage) {
        when (message) {
            is ControlMessage.JoinRequest -> handleJoinRequestMessage(message)
            is ControlMessage.JoinApproval -> handleJoinApprovalMessage(message)
            is ControlMessage.JoinRejection -> handleJoinRejectionMessage(message)
            is ControlMessage.StreamStart -> handleRemoteStreamStart(message)
            is ControlMessage.Pause -> handleRemotePause(message)
            is ControlMessage.Stop -> handleRemoteStop(message)
            is ControlMessage.Disconnect -> handleRemoteDisconnect(message)
            is ControlMessage.Heartbeat -> wifiDirectService.recordHeartbeat()
            is ControlMessage.ResyncNotice -> {
                if (message.listenerId == localListenerDeviceId) {
                    _uiState.value = _uiState.value.copy(lastMessage = message.reason)
                }
            }
            is ControlMessage.Hello -> Unit
        }
    }

    private fun handleJoinRequestMessage(message: ControlMessage.JoinRequest) {
        if (message.sessionId != currentSessionId) return
        val request = JoinRequest(
            requestId = "${message.device.deviceId}-${message.sessionId.value}",
            sessionId = message.sessionId.value,
            listenerId = message.device.deviceId,
            listenerName = message.device.displayName,
            inviteCode = message.inviteCode,
            requestedAtMs = SystemClock.elapsedRealtime(),
        )
        if (_uiState.value.pendingJoinRequests.any { it.listenerId == request.listenerId }) return
        _uiState.value = _uiState.value.copy(
            pendingJoinRequests = _uiState.value.pendingJoinRequests + request,
            hostState = HostLifecycleState.READY,
            lastMessage = "${request.listenerName} requested to join",
            lastError = null,
        )
        refreshHostDiagnostics()
    }

    private fun handleJoinApprovalMessage(message: ControlMessage.JoinApproval) {
        if (message.listenerId != localListenerDeviceId) return
        _uiState.value = _uiState.value.copy(
            listenerState = ListenerLifecycleState.APPROVED,
            connectionProgress = _uiState.value.connectionProgress.copy(
                currentState = ListenerLifecycleState.APPROVED,
                approved = true,
                connected = true,
            ),
            lastMessage = "Join approved",
            lastError = null,
        )
        if (message.trustedForFuture) {
            preferences.edit { putBoolean("trusted:${message.listenerId}", true) }
        }
        refreshListenerDiagnostics()
        manualResync()
    }

    private fun handleJoinRejectionMessage(message: ControlMessage.JoinRejection) {
        if (message.listenerId != localListenerDeviceId) return
        handleListenerConnectionFailure(message.reason)
    }

    private fun handleRemoteStreamStart(message: ControlMessage.StreamStart) {
        if (_uiState.value.selectedSession?.id != message.sessionId.value) return
        currentStreamId = message.streamId
        _uiState.value = _uiState.value.copy(
            listenerState = ListenerLifecycleState.BUFFERING,
            listenerPlaybackState = PlaybackState.BUFFERING,
            connectionProgress = _uiState.value.connectionProgress.copy(
                currentState = ListenerLifecycleState.BUFFERING,
                approved = true,
                connected = true,
                synced = _uiState.value.listenerSyncState.confidence != SyncQualityBadge.UNKNOWN,
            ),
            lastMessage = "Host stream starting",
            lastError = null,
        )
        startTransportListenerPlayback(message.sessionId, message.streamId)
    }

    private fun handleRemotePause(message: ControlMessage.Pause) {
        if (_uiState.value.selectedSession?.id != message.sessionId.value) return
        propagateListenerPlaybackState(
            playbackState = PlaybackState.PAUSED,
            listenerState = _uiState.value.listenerState,
            message = "Host paused the stream",
        )
    }

    private fun handleRemoteStop(message: ControlMessage.Stop) {
        if (_uiState.value.selectedSession?.id != message.sessionId.value) return
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

    private fun handleRemoteDisconnect(message: ControlMessage.Disconnect) {
        if (_uiState.value.selectedSession?.id != message.sessionId.value) return
        if (message.listenerId != localListenerDeviceId && message.listenerId.isNotBlank()) return
        handleListenerDisconnect(message.reason)
    }

    private fun applySyncResponse(response: SyncResponsePacket) {
        if (_uiState.value.selectedSession?.id != response.sessionId.value) return
        val expectedCorrelationId = pendingSyncCorrelationId
        if (expectedCorrelationId != null && response.correlationId != expectedCorrelationId) return
        pendingSyncCorrelationId = null
        val controller = listenerSyncController ?: ListenerSyncController(response.sessionId).also {
            listenerSyncController = it
        }
        val syncState = controller.onResponse(response).copy(
            resyncCount = _uiState.value.listenerSyncState.resyncCount + 1,
        )
        val shouldResync = controller.shouldResync(state = syncState)
        logger.i(
            "sync.sample",
            "offset=${"%.2f".format(syncState.offsetMs)} rtt=${"%.2f".format(syncState.rttMs)} jitter=${"%.2f".format(syncState.jitterMs)}",
        )
        if (shouldResync && !_uiState.value.connectionProgress.synced) {
            handleSyncFailure("Unable to establish a stable sync estimate")
            return
        }
        _uiState.value = _uiState.value.copy(
            listenerSyncState = syncState,
            listenerState = if (shouldResync) ListenerLifecycleState.DESYNCED else _uiState.value.listenerState,
            connectionProgress = _uiState.value.connectionProgress.copy(synced = !shouldResync),
        )
        diagnosticsStore.updateListener {
            it.copy(
                hostOffsetMs = syncState.offsetMs,
                rttMs = syncState.rttMs,
                jitterMs = syncState.jitterMs,
                resyncCount = syncState.resyncCount,
                metricsSummary = summarizeMetrics(),
                lastError = if (shouldResync) "Sync drift exceeded threshold" else null,
            )
        }
        metrics.increment("sync_sample")
        metrics.recordTiming("sync_rtt_ms", syncState.rttMs)
        if (shouldResync) {
            metrics.increment("playback_desync")
            logger.w("playback.desync", "Resync threshold exceeded")
        }
        refreshListenerDiagnostics()
        refreshHostDiagnostics()
    }

    private fun handleIncomingAudioPacket(packet: AudioPacket) {
        if (_uiState.value.selectedSession?.id != packet.sessionId.value) return
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

    private fun recordIncomingPacket(scheduler: ListenerPlaybackScheduler, packet: AudioPacket) {
        val telemetry = scheduler.submit(packet)
        if (telemetry.lateDropCount > 0) {
            logger.w("packet.receive.anomaly", "Late packet dropped seq=${packet.sequenceNumber}")
        }
        diagnosticsStore.updateListener {
            it.copy(
                sessionId = packet.sessionId.value,
                packetLossCount = telemetry.packetLossCount,
                lateDropCount = telemetry.lateDropCount,
                invalidPacketCount = telemetry.invalidPacketCount,
                concealedPacketCount = telemetry.concealedPacketCount,
                bufferDepthMs = telemetry.bufferDepthMs,
                lastPacketSequence = packet.sequenceNumber,
                endOfStreamReached = false,
            )
        }
        refreshListenerDiagnostics()
    }

    private fun startListenerPlaybackSimulation(sessionId: String) {
        val packets = if (latestPackets.isNotEmpty()) {
            latestPackets.take(24)
        } else {
            generateSyntheticPackets(sessionId)
        }
        val expectedStreamId = packets.firstOrNull()?.streamId ?: currentStreamId ?: StreamId("synthetic-stream")
        val mapper = HostTimeMapper(offsetMs = _uiState.value.listenerSyncState.offsetMs, skewPpm = _uiState.value.listenerSyncState.skewPpm)
        listenerScheduler = ListenerPlaybackScheduler(
            mapper = mapper,
            thresholds = currentPlaybackThresholds(),
            expectedSessionId = SessionId(sessionId),
            expectedStreamId = expectedStreamId,
        )
        packets.forEach { packet -> listenerScheduler?.let { recordIncomingPacket(it, packet) } }
        playbackJob?.cancel()
        playbackJob = viewModelScope.launch {
            var lastUnderrunCount = 0
            delay(300)
            _uiState.value = _uiState.value.copy(
                listenerState = ListenerLifecycleState.PLAYING,
                listenerPlaybackState = PlaybackState.PLAYING,
                connectionProgress = _uiState.value.connectionProgress.copy(
                    currentState = ListenerLifecycleState.PLAYING,
                    playing = true,
                ),
            )
            while (listenerScheduler?.canStart() == true) {
                if (wifiDirectService.snapshot.value.state == TransportConnectionState.DISCONNECTED ||
                    wifiDirectService.snapshot.value.state == TransportConnectionState.FAILED
                ) {
                    handleListenerDisconnect("Transport disconnected during playback")
                    return@launch
                }
                val frame = listenerScheduler?.poll() ?: break
                playbackEngine.write(frame)
                val telemetry = listenerScheduler?.snapshot() ?: break
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
                delay(20)
            }
            diagnosticsStore.updateListener {
                it.copy(
                    playbackState = PlaybackState.STOPPED,
                    endOfStreamReached = true,
                    metricsSummary = summarizeMetrics(),
                )
            }
            _uiState.value = _uiState.value.copy(
                listenerPlaybackState = PlaybackState.STOPPED,
                lastMessage = "Reached end of file",
            )
            refreshListenerDiagnostics()
        }
        startPeriodicResync()
    }

    private fun startTransportListenerPlayback(sessionId: SessionId, streamId: StreamId) {
        val mapper = HostTimeMapper(
            offsetMs = _uiState.value.listenerSyncState.offsetMs,
            skewPpm = _uiState.value.listenerSyncState.skewPpm,
        )
        listenerScheduler = ListenerPlaybackScheduler(
            mapper = mapper,
            thresholds = currentPlaybackThresholds(),
            expectedSessionId = sessionId,
            expectedStreamId = streamId,
        )
        pendingTransportPackets
            .filter { it.sessionId == sessionId && it.streamId == streamId }
            .forEach { packet -> listenerScheduler?.let { recordIncomingPacket(it, packet) } }
        pendingTransportPackets.clear()
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
                val scheduler = listenerScheduler ?: return@launch
                if (!started) {
                    if (!scheduler.canStart()) {
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
                            playing = true,
                        ),
                    )
                }
                val frame = scheduler.poll()
                if (frame == null) {
                    delay(10)
                    continue
                }
                playbackEngine.write(frame)
                val telemetry = scheduler.snapshot()
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

    private fun startHostStreamingLoop(streamId: StreamId) {
        hostStreamJob?.cancel()
        hostStreamJob = viewModelScope.launch {
            var previousSendElapsedMs: Long? = null
            val packetDurationMs = latestPackets.firstOrNull()?.let { it.samplesPerPacket * 1_000L / it.sampleRate } ?: 20L
            latestPackets.forEachIndexed { index, packet ->
                val now = SystemClock.elapsedRealtime()
                previousSendElapsedMs?.let { previous ->
                    val sendGap = now - previous
                    metrics.recordTiming("packet_gap_ms", sendGap.toDouble())
                    if (kotlin.math.abs(sendGap - packetDurationMs) > 8) {
                        metrics.increment("packet_send_anomaly")
                        logger.w("packet.send.anomaly", "seq=${packet.sequenceNumber} gap=${sendGap}ms")
                    }
                }
                if (index % 25 == 0 || index == latestPackets.lastIndex) {
                    logger.i("stream.packet", "stream=${streamId.value} sent=${index + 1}/${latestPackets.size}")
                }
                metrics.increment("packet_send_total")
                diagnosticsStore.updateHost {
                    it.copy(
                        packetSendCount = (index + 1).toLong(),
                        packetSendRatePerSecond = 1_000.0 / packetDurationMs,
                        lastContactElapsedMs = now,
                        metricsSummary = summarizeMetrics(),
                    )
                }
                playbackEngine.write(
                    PlaybackFrame(
                        packet = packet,
                        localDeadlineMs = packet.hostPresentationTimeMs,
                    ),
                )
                runCatching {
                    wifiDirectService.broadcastAudio(packet)
                }.onFailure { error ->
                    logger.w("transport.audio", "Failed to send packet ${packet.sequenceNumber}: ${error.message}")
                }
                refreshHostDiagnostics()
                previousSendElapsedMs = now
                delay(packetDurationMs)
            }
            logger.i("stream.stop", "Reached end of file for host stream")
            metrics.increment("stream_eof")
            _uiState.value = _uiState.value.copy(
                hostState = HostLifecycleState.READY,
                hostPlaybackState = PlaybackState.STOPPED,
                lastMessage = "Reached end of file",
            )
            propagateListenerPlaybackState(
                playbackState = PlaybackState.STOPPED,
                listenerState = _uiState.value.listenerState,
                message = "Host stream reached end of file",
            )
            currentSessionId?.let { sessionId ->
                viewModelScope.launch {
                    runCatching {
                        wifiDirectService.broadcastControl(
                            ControlMessage.Stop(
                                version = 1,
                                sessionId = sessionId,
                                streamId = streamId,
                                hostStopTimeMs = SystemClock.elapsedRealtime(),
                            ),
                        )
                    }
                }
            }
            refreshHostDiagnostics(streamState = PlaybackState.STOPPED)
        }
    }

    private fun propagateListenerPlaybackState(
        playbackState: PlaybackState,
        listenerState: ListenerLifecycleState,
        message: String,
    ) {
        _uiState.value = _uiState.value.copy(
            listenerPlaybackState = playbackState,
            listenerState = listenerState,
            connectionProgress = _uiState.value.connectionProgress.copy(
                playing = playbackState == PlaybackState.PLAYING,
            ),
            lastMessage = message,
        )
        diagnosticsStore.updateListener {
            it.copy(
                playbackState = playbackState,
                metricsSummary = summarizeMetrics(),
                lastError = if (listenerState == ListenerLifecycleState.DESYNCED) {
                    "Listener sync trouble detected"
                } else {
                    it.lastError
                },
            )
        }
        refreshListenerDiagnostics()
    }

    private fun handleListenerConnectionFailure(message: String) {
        logger.w("transport.error", message)
        _uiState.value = _uiState.value.copy(
            listenerState = ListenerLifecycleState.ERROR,
            listenerPlaybackState = PlaybackState.ERROR,
            lastError = message,
        )
        diagnosticsStore.updateListener {
            it.copy(
                playbackState = PlaybackState.ERROR,
                lastError = message,
                metricsSummary = summarizeMetrics(),
            )
        }
        refreshListenerDiagnostics()
    }

    private fun handleSyncFailure(message: String) {
        logger.w("sync.error", message)
        metrics.increment("sync_establish_failure")
        _uiState.value = _uiState.value.copy(
            listenerState = ListenerLifecycleState.ERROR,
            listenerPlaybackState = PlaybackState.ERROR,
            lastError = message,
        )
        diagnosticsStore.updateListener {
            it.copy(
                playbackState = PlaybackState.ERROR,
                lastError = message,
                metricsSummary = summarizeMetrics(),
            )
        }
        refreshListenerDiagnostics()
    }

    private fun handleListenerDisconnect(message: String) {
        logger.w("transport.disconnect", message)
        _uiState.value = _uiState.value.copy(
            listenerState = ListenerLifecycleState.DISCONNECTED,
            listenerPlaybackState = PlaybackState.STOPPED,
            lastError = message,
        )
        diagnosticsStore.updateListener {
            it.copy(
                playbackState = PlaybackState.STOPPED,
                lastError = message,
                metricsSummary = summarizeMetrics(),
            )
        }
        refreshListenerDiagnostics()
    }

    private fun startPeriodicResync() {
        resyncJob?.cancel()
        resyncJob = viewModelScope.launch {
            while (shouldKeepResyncing()) {
                delay(_uiState.value.tuningSettings.syncCadenceMs)
                manualResync()
                wifiDirectService.recordHeartbeat()
                if (wifiDirectService.snapshot.value.state == TransportConnectionState.DISCONNECTED ||
                    wifiDirectService.snapshot.value.state == TransportConnectionState.FAILED
                ) {
                    handleListenerDisconnect("Transport disconnected during playback")
                    return@launch
                }
                refreshHostDiagnostics()
            }
        }
    }

    private fun refreshHostDiagnostics(
        streamState: PlaybackState = _uiState.value.hostPlaybackState,
        sessionId: String = _uiState.value.hostDiagnostics.sessionId,
    ) {
        diagnosticsStore.updateHost {
            it.copy(
                sessionId = sessionId,
                listenerCount = _uiState.value.approvedListeners.size + _uiState.value.pendingJoinRequests.size,
                pendingJoinCount = _uiState.value.pendingJoinRequests.size,
                connectedListenerCount = _uiState.value.approvedListeners.count {
                    it.connectionState == TransportConnectionState.CONNECTING || it.connectionState == TransportConnectionState.CONNECTED
                },
                desyncedListenerCount = _uiState.value.approvedListeners.count {
                    it.listenerState == ListenerLifecycleState.DESYNCED || it.syncQuality == SyncQualityBadge.POOR
                } + if (_uiState.value.listenerState == ListenerLifecycleState.DESYNCED) 1 else 0,
                streamState = streamState,
                lastContactElapsedMs = wifiDirectService.snapshot.value.lastContactElapsedMs,
                metricsSummary = summarizeMetrics(),
                packetBudgetSummary = it.packetBudgetSummary,
                lastError = _uiState.value.lastError,
            )
        }
        _uiState.value = _uiState.value.copy(hostDiagnostics = diagnosticsStore.hostDiagnostics.value)
    }

    private fun refreshListenerDiagnostics() {
        _uiState.value = _uiState.value.copy(listenerDiagnostics = diagnosticsStore.listenerDiagnostics.value)
    }

    private fun generateSyntheticPackets(sessionId: String): List<AudioPacket> {
        val packetizer = PcmPacketizer(
            sessionId = SessionId(sessionId),
            streamId = StreamId("synthetic-stream"),
            format = AudioFormatSpec(),
        )
        return packetizer.packetize(
            chunk = DecodedAudioChunk(
                pcm16Le = ByteArray(48_000 / 25 * 4 * 8),
                firstSampleIndex = 0,
                frameCount = 48_000 / 25 * 8,
            ),
            hostPresentationStartMs = SystemClock.elapsedRealtime() + 500,
        )
    }

    private fun summarizeMetrics(): String {
        val counters = metrics.snapshotCounters()
        val timings = metrics.snapshotTimings()
        if (counters.isEmpty() && timings.isEmpty()) return "No metrics yet"
        val counterSummary = counters.entries.joinToString(", ") { "${it.key}=${it.value}" }
        val timingSummary = timings.entries.joinToString(", ") { "${it.key}=${"%.1f".format(it.value)}ms" }
        return listOf(counterSummary, timingSummary).filter { it.isNotBlank() }.joinToString(" | ")
    }

    private fun generateInviteCode(): String =
        ((SystemClock.elapsedRealtime() % 9_000) + 1_000).toString()

    private fun refreshDiscoveredSessions() {
        val bleSessions = bleService.discoveredSessions.value
        val peerSessions = wifiDirectService.snapshot.value.peers.map {
            SessionInfo(
                id = "p2p-${it.deviceAddress.replace(":", "").lowercase()}",
                name = "Nearby session (${it.deviceName})",
                hostDeviceName = it.deviceName,
                approvalMode = ApprovalMode.MANUAL,
                inviteCodeRequired = false,
            )
        }
        val merged = (bleSessions + peerSessions)
            .distinctBy { it.id }
            .sortedBy { it.name }
        _uiState.value = _uiState.value.copy(
            discoveredSessions = merged,
            connectionProgress = _uiState.value.connectionProgress.copy(discovered = merged.isNotEmpty()),
        )
    }

    private fun sendPendingJoinRequest() {
        val request = pendingJoinRequestMessage ?: return
        viewModelScope.launch {
            runCatching {
                wifiDirectService.sendControlToHost(request)
            }.onSuccess {
                pendingJoinRequestMessage = null
                _uiState.value = _uiState.value.copy(
                    listenerState = ListenerLifecycleState.AWAITING_APPROVAL,
                    connectionProgress = _uiState.value.connectionProgress.copy(
                        currentState = ListenerLifecycleState.AWAITING_APPROVAL,
                        connected = true,
                    ),
                    lastMessage = "Join request sent",
                    lastError = null,
                )
            }.onFailure { error ->
                handleListenerConnectionFailure(error.message ?: "Failed to send join request")
            }
        }
    }

    private fun currentPlaybackThresholds(): PlaybackThresholds = PlaybackThresholds(
        startupBufferMs = _uiState.value.tuningSettings.startupBufferMs,
        softCorrectionThresholdMs = _uiState.value.tuningSettings.latePacketThresholdMs,
        hardResyncThresholdMs = _uiState.value.tuningSettings.hardResyncThresholdMs,
    )

    private fun createSyncController(sessionId: SessionId): ListenerSyncController {
        val tuning = _uiState.value.tuningSettings
        return ListenerSyncController(
            sessionId = sessionId,
            estimator = ClockSyncEstimator(maxSamples = tuning.syncSampleWindow),
            config = SyncMaintenanceConfig(
                cadenceMs = tuning.syncCadenceMs,
                driftThresholdMs = tuning.syncDriftThresholdMs,
                sampleHistorySize = tuning.syncSampleWindow,
            ),
        )
    }

    private fun shouldKeepResyncing(): Boolean =
        _uiState.value.hostPlaybackState == PlaybackState.PLAYING ||
            _uiState.value.listenerState in setOf(
                ListenerLifecycleState.APPROVED,
                ListenerLifecycleState.CONNECTING,
                ListenerLifecycleState.SYNCING_CLOCK,
                ListenerLifecycleState.BUFFERING,
                ListenerLifecycleState.PLAYING,
                ListenerLifecycleState.DESYNCED,
            )

    private fun loadTuningSettings(): TuningSettings = TuningSettings(
        syncSampleWindow = preferences.getInt("tuning:syncSampleWindow", 12),
        syncCadenceMs = preferences.getLong("tuning:syncCadenceMs", 2_000L),
        startupBufferMs = preferences.getLong("tuning:startupBufferMs", 400L),
        latePacketThresholdMs = preferences.getLong("tuning:latePacketThresholdMs", 40L),
        hardResyncThresholdMs = preferences.getLong("tuning:hardResyncThresholdMs", 120L),
        syncDriftThresholdMs = java.lang.Double.longBitsToDouble(
            preferences.getLong("tuning:syncDriftThresholdBits", java.lang.Double.doubleToLongBits(18.0)),
        ),
    )

    private fun persistTuningSettings(settings: TuningSettings) {
        preferences.edit {
            putInt("tuning:syncSampleWindow", settings.syncSampleWindow)
            putLong("tuning:syncCadenceMs", settings.syncCadenceMs)
            putLong("tuning:startupBufferMs", settings.startupBufferMs)
            putLong("tuning:latePacketThresholdMs", settings.latePacketThresholdMs)
            putLong("tuning:hardResyncThresholdMs", settings.hardResyncThresholdMs)
            putLong("tuning:syncDriftThresholdBits", java.lang.Double.doubleToLongBits(settings.syncDriftThresholdMs))
        }
    }

    private fun hasPermission(permission: AppPermission): Boolean =
        _uiState.value.permissions.any { it.permission == permission && it.granted }

    private fun hasHostTransportPermissions(): Boolean =
        hasPermission(AppPermission.NearbyWifiDevices) &&
            hasPermission(AppPermission.ChangeWifiState) &&
            hasPermission(AppPermission.BluetoothAdvertise)

    private fun hasListenerTransportPermissions(): Boolean =
        hasPermission(AppPermission.NearbyWifiDevices) &&
            hasPermission(AppPermission.BluetoothScan) &&
            hasPermission(AppPermission.BluetoothConnect)

    private fun demoSessions(): List<SessionInfo> = listOf(
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
        super.onCleared()
    }
}
