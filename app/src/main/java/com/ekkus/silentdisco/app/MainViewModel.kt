package com.ekkus.silentdisco.app

import android.app.Application
import android.content.Context
import android.net.Uri
import android.os.SystemClock
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
import com.ekkus.silentdisco.core.audio.PlaybackThresholds
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
import com.ekkus.silentdisco.core.model.SyncState
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.model.TrustState
import com.ekkus.silentdisco.core.permissions.PermissionCatalogue
import com.ekkus.silentdisco.core.permissions.PermissionState
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.ControlMessage
import com.ekkus.silentdisco.core.protocol.DeviceIdentity
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.sync.HostTimeMapper
import com.ekkus.silentdisco.core.sync.HostTimingService
import com.ekkus.silentdisco.core.sync.ListenerSyncController
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
            discoveredSessions = demoSessions(),
        ),
    )
    val uiState: StateFlow<AppUiState> = _uiState.asStateFlow()

    private var currentSessionId: SessionId? = null
    private var currentStreamId: StreamId? = null
    private var listenerSyncController: ListenerSyncController? = null
    private var listenerScheduler: ListenerPlaybackScheduler? = null
    private var latestDecodedAudio: AudioDecodeResult? = null
    private var latestPackets: List<AudioPacket> = emptyList()
    private var playbackJob: Job? = null
    private var resyncJob: Job? = null

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

    fun createHostSession(): Boolean {
        val form = _uiState.value.hostForm
        if (form.sessionName.isBlank()) {
            _uiState.value = _uiState.value.copy(lastError = "Session name is required")
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
        bleService.startAdvertising(
            BleAdvertisement(
                sessionId = session.id,
                sessionName = session.name,
                hostName = session.hostDeviceName,
                approvalRequired = true,
            ),
        )
        wifiDirectService.startHost(session)
        logger.i("transport.host", "Started BLE discovery bridge for ${session.id}")
        diagnosticsStore.updateHost {
            it.copy(
                sessionId = session.id,
                streamState = PlaybackState.STOPPED,
                lastContactElapsedMs = SystemClock.elapsedRealtime(),
            )
        }
        _uiState.value = _uiState.value.copy(
            hostState = HostLifecycleState.WAITING_FOR_LISTENERS,
            discoveredSessions = (listOf(session) + demoSessions()).distinctBy { it.id },
            lastMessage = "Hosting ${session.name}",
            lastError = null,
        )
        refreshHostDiagnostics()
        return true
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
        refreshHostDiagnostics()
        refreshListenerDiagnostics()
    }

    fun trustListener(listenerId: String) {
        preferences.edit().putBoolean("trusted:$listenerId", true).apply()
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
            metrics.increment("stream_start")
            metrics.increment("packet_send_total", latestPackets.size.toLong())
            metrics.recordTiming("packet_duration_ms", 20.0)
            diagnosticsStore.updateHost {
                it.copy(
                    packetSendCount = latestPackets.size.toLong(),
                    packetSendRatePerSecond = 50.0,
                    streamState = PlaybackState.PLAYING,
                    lastContactElapsedMs = SystemClock.elapsedRealtime(),
                    metricsSummary = summarizeMetrics(),
                    lastError = null,
                )
            }
            _uiState.value = _uiState.value.copy(
                hostState = HostLifecycleState.STREAMING,
                hostPlaybackState = PlaybackState.PLAYING,
                lastMessage = "Host stream started via ${playbackEngine.start()}",
                lastError = null,
            )
            refreshHostDiagnostics()
            startPeriodicResync()
        }.onFailure { error ->
            metrics.increment("stream_start_error")
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
        _uiState.value = _uiState.value.copy(
            hostState = HostLifecycleState.PAUSED,
            hostPlaybackState = PlaybackState.PAUSED,
        )
        metrics.increment("stream_pause")
        refreshHostDiagnostics(streamState = PlaybackState.PAUSED)
    }

    fun stopHostPlayback() {
        playbackJob?.cancel()
        resyncJob?.cancel()
        playbackEngine.stop()
        _uiState.value = _uiState.value.copy(
            hostState = HostLifecycleState.READY,
            hostPlaybackState = PlaybackState.STOPPED,
        )
        metrics.increment("stream_stop")
        refreshHostDiagnostics(streamState = PlaybackState.STOPPED)
    }

    fun endSession() {
        stopHostPlayback()
        bleService.stop()
        wifiDirectService.stop()
        _uiState.value = _uiState.value.copy(
            hostState = HostLifecycleState.IDLE,
            pendingJoinRequests = emptyList(),
            approvedListeners = emptyList(),
            hostPlaybackState = PlaybackState.STOPPED,
            lastMessage = "Session ended",
        )
        refreshHostDiagnostics(streamState = PlaybackState.STOPPED, sessionId = "")
    }

    fun scanForSessions() {
        logger.i("listener.scan", "Scanning for nearby sessions")
        bleService.startScanning()
        wifiDirectService.discoverPeers()
        val discovered = bleService.discoveredSessions.value.ifEmpty { demoSessions() }
        _uiState.value = _uiState.value.copy(
            listenerState = if (discovered.isEmpty()) ListenerLifecycleState.ERROR else ListenerLifecycleState.SCANNING,
            discoveredSessions = discovered,
            connectionProgress = _uiState.value.connectionProgress.copy(
                currentState = if (discovered.isEmpty()) ListenerLifecycleState.ERROR else ListenerLifecycleState.SCANNING,
                discovered = discovered.isNotEmpty(),
            ),
            lastError = if (discovered.isEmpty()) "No sessions found nearby" else null,
        )
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
        if (session.inviteCodeRequired && _uiState.value.connectionProgress.inviteCode.isBlank()) {
            _uiState.value = _uiState.value.copy(lastError = "Invite code required")
            return
        }
        val request = ControlMessage.JoinRequest(
            version = 1,
            sessionId = SessionId(session.id),
            device = DeviceIdentity("listener-device", "This Android Listener"),
            inviteCode = _uiState.value.connectionProgress.inviteCode.ifBlank { null },
        )
        logger.i("listener.join", "Join request created for ${request.sessionId.value}")
        _uiState.value = _uiState.value.copy(
            listenerState = ListenerLifecycleState.AWAITING_APPROVAL,
            connectionProgress = _uiState.value.connectionProgress.copy(
                currentState = ListenerLifecycleState.AWAITING_APPROVAL,
                requested = true,
            ),
            lastMessage = "Join request sent",
            lastError = null,
        )
        val shouldReject = session.inviteCodeRequired && request.inviteCode != "1234"
        simulateApprovalAndPlayback(session.id, shouldReject)
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

    fun leaveSession() {
        playbackJob?.cancel()
        resyncJob?.cancel()
        listenerScheduler = null
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
            ListenerSyncController(SessionId(it.id))
        } ?: return
        listenerSyncController = controller
        val request = controller.newProbe()
        val response = hostTimingService.createResponse(request)
        val syncState = controller.onResponse(response).copy(
            resyncCount = _uiState.value.listenerSyncState.resyncCount + 1,
        )
        val shouldResync = controller.shouldResync(state = syncState)
        _uiState.value = _uiState.value.copy(
            listenerSyncState = syncState,
            listenerState = if (shouldResync) ListenerLifecycleState.DESYNCED else _uiState.value.listenerState,
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
        refreshListenerDiagnostics()
        refreshHostDiagnostics()
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

    private fun startListenerPlaybackSimulation(sessionId: String) {
        val mapper = HostTimeMapper(offsetMs = _uiState.value.listenerSyncState.offsetMs, skewPpm = _uiState.value.listenerSyncState.skewPpm)
        listenerScheduler = ListenerPlaybackScheduler(
            mapper = mapper,
            thresholds = PlaybackThresholds(),
        )
        val packets = if (latestPackets.isNotEmpty()) {
            latestPackets.take(24)
        } else {
            generateSyntheticPackets(sessionId)
        }
        packets.forEach { packet ->
            val telemetry = listenerScheduler?.submit(packet)
            diagnosticsStore.updateListener {
                it.copy(
                    sessionId = sessionId,
                    packetLossCount = telemetry?.packetLossCount ?: it.packetLossCount,
                    lateDropCount = telemetry?.lateDropCount ?: it.lateDropCount,
                    bufferDepthMs = telemetry?.bufferDepthMs ?: it.bufferDepthMs,
                    lastPacketSequence = packet.sequenceNumber,
                )
            }
        }
        playbackJob?.cancel()
        playbackJob = viewModelScope.launch {
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
                val frame = listenerScheduler?.poll() ?: break
                playbackEngine.write(frame)
                val telemetry = listenerScheduler?.snapshot() ?: break
                diagnosticsStore.updateListener {
                    it.copy(
                        playbackState = if (telemetry.underrunCount > 0) PlaybackState.UNDERRUN else PlaybackState.PLAYING,
                        playbackPositionMs = playbackEngine.playbackPositionMs(frame),
                        bufferDepthMs = telemetry.bufferDepthMs,
                        packetLossCount = telemetry.packetLossCount,
                        lateDropCount = telemetry.lateDropCount,
                        underrunCount = telemetry.underrunCount,
                        lastPacketSequence = telemetry.lastPlayedSequence,
                        metricsSummary = summarizeMetrics(),
                    )
                }
                if (telemetry.shouldResync) {
                    _uiState.value = _uiState.value.copy(listenerState = ListenerLifecycleState.DESYNCED)
                }
                delay(20)
            }
            refreshListenerDiagnostics()
        }
        startPeriodicResync()
    }

    private fun startPeriodicResync() {
        resyncJob?.cancel()
        resyncJob = viewModelScope.launch {
            repeat(3) {
                delay(2_000)
                manualResync()
                wifiDirectService.recordHeartbeat()
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
                streamState = streamState,
                lastContactElapsedMs = wifiDirectService.snapshot.value.lastContactElapsedMs,
                metricsSummary = summarizeMetrics(),
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
        if (counters.isEmpty()) return "No metrics yet"
        return counters.entries.joinToString(", ") { "${it.key}=${it.value}" }
    }

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
}
