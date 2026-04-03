package com.ekkus.silentdisco.app

import android.app.Application
import android.net.Uri
import android.os.SystemClock
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.core.audio.AudioFormatSpec
import com.ekkus.silentdisco.core.audio.AudioPacketBuffer
import com.ekkus.silentdisco.core.audio.BufferedAudioPacket
import com.ekkus.silentdisco.core.audio.DecodedAudioChunk
import com.ekkus.silentdisco.core.audio.OboeBridge
import com.ekkus.silentdisco.core.audio.PcmPacketizer
import com.ekkus.silentdisco.core.diagnostics.DiagnosticsStore
import com.ekkus.silentdisco.core.logging.AppLogger
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
import com.ekkus.silentdisco.core.permissions.PermissionCatalogue
import com.ekkus.silentdisco.core.permissions.PermissionState
import com.ekkus.silentdisco.core.protocol.DeviceIdentity
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.sync.ClockSyncEstimator
import com.ekkus.silentdisco.core.sync.ClockSyncSample
import com.ekkus.silentdisco.core.transport.BleAdvertisement
import com.ekkus.silentdisco.core.transport.BleDiscoveryService
import com.ekkus.silentdisco.core.transport.WifiDirectTransportService
import java.util.UUID
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

class MainViewModel(application: Application) : AndroidViewModel(application) {
    private val logger = AppLogger()
    private val diagnosticsStore = DiagnosticsStore()
    private val bleService = BleDiscoveryService(application)
    private val wifiDirectService = WifiDirectTransportService(application)
    private val syncEstimator = ClockSyncEstimator()
    private val packetBuffer = AudioPacketBuffer()

    private val _uiState = MutableStateFlow(
        AppUiState(
            permissions = PermissionCatalogue.requiredPermissions().map {
                PermissionState(permission = it, granted = false)
            },
            discoveredSessions = demoSessions(),
        ),
    )
    val uiState: StateFlow<AppUiState> = _uiState.asStateFlow()

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
        val selected = SelectedAudioFile(
            uri = uri,
            displayName = displayName,
            mimeType = mimeType,
            sizeBytes = null,
        )
        _uiState.value = _uiState.value.copy(
            hostForm = _uiState.value.hostForm.copy(selectedAudio = selected),
            lastMessage = "Selected $displayName",
            lastError = null,
        )
    }

    fun createHostSession(): Boolean {
        val form = _uiState.value.hostForm
        if (form.sessionName.isBlank()) {
            _uiState.value = _uiState.value.copy(lastError = "Session name is required")
            return false
        }
        val sessionId = UUID.randomUUID().toString()
        val session = SessionInfo(
            id = sessionId,
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
        wifiDirectService.startHost()
        diagnosticsStore.updateHost {
            it.copy(
                sessionId = session.id,
                streamState = PlaybackState.STOPPED,
            )
        }
        _uiState.value = _uiState.value.copy(
            hostState = HostLifecycleState.WAITING_FOR_LISTENERS,
            discoveredSessions = (listOf(session) + demoSessions()).distinctBy { it.id },
            lastMessage = "Hosting ${session.name}",
            lastError = null,
        )
        return true
    }

    fun addDemoJoinRequest() {
        val sessionId = _uiState.value.hostDiagnostics.sessionId.ifBlank { "demo-session" }
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
        _uiState.value = _uiState.value.copy(
            pendingJoinRequests = _uiState.value.pendingJoinRequests - request,
            approvedListeners = _uiState.value.approvedListeners + request.toListenerInfo(),
            lastMessage = "${request.listenerName} approved",
        )
        refreshHostDiagnostics()
    }

    fun rejectJoinRequest(request: JoinRequest) {
        _uiState.value = _uiState.value.copy(
            pendingJoinRequests = _uiState.value.pendingJoinRequests - request,
            lastMessage = "${request.listenerName} rejected",
        )
        refreshHostDiagnostics()
    }

    fun startHostPlayback() {
        val sessionId = _uiState.value.hostDiagnostics.sessionId.ifBlank { UUID.randomUUID().toString() }
        val packets = PcmPacketizer(
            sessionId = SessionId(sessionId),
            streamId = StreamId("stream-1"),
            format = AudioFormatSpec(),
        ).packetize(
            chunk = DecodedAudioChunk(
                pcm16Le = ByteArray(48_000 / 25 * 4),
                firstSampleIndex = 0,
                frameCount = 48_000 / 25,
            ),
            hostPresentationStartMs = SystemClock.elapsedRealtime() + 500,
        )
        packets.take(20).forEach { packet ->
            packetBuffer.insert(
                BufferedAudioPacket(
                    packet = packet,
                    scheduledLocalTimeMs = packet.hostPresentationTimeMs,
                ),
            )
        }
        diagnosticsStore.updateHost {
            it.copy(
                packetSendCount = it.packetSendCount + packets.size,
                packetSendRatePerSecond = 50.0,
                streamState = PlaybackState.PLAYING,
            )
        }
        _uiState.value = _uiState.value.copy(
            hostState = HostLifecycleState.STREAMING,
            hostPlaybackState = PlaybackState.PLAYING,
            lastMessage = "Host stream started via ${OboeBridge.backendSummary()}",
        )
        refreshHostDiagnostics()
    }

    fun pauseHostPlayback() {
        _uiState.value = _uiState.value.copy(
            hostState = HostLifecycleState.PAUSED,
            hostPlaybackState = PlaybackState.PAUSED,
        )
        refreshHostDiagnostics(streamState = PlaybackState.PAUSED)
    }

    fun stopHostPlayback() {
        _uiState.value = _uiState.value.copy(
            hostState = HostLifecycleState.READY,
            hostPlaybackState = PlaybackState.STOPPED,
        )
        refreshHostDiagnostics(streamState = PlaybackState.STOPPED)
    }

    fun endSession() {
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
        _uiState.value = _uiState.value.copy(
            listenerState = ListenerLifecycleState.SCANNING,
            discoveredSessions = bleService.discoveredSessions.value.ifEmpty { demoSessions() },
            connectionProgress = _uiState.value.connectionProgress.copy(
                currentState = ListenerLifecycleState.SCANNING,
                discovered = true,
            ),
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
        val session = _uiState.value.selectedSession ?: return
        val requiresCode = session.inviteCodeRequired
        if (requiresCode && _uiState.value.connectionProgress.inviteCode.isBlank()) {
            _uiState.value = _uiState.value.copy(lastError = "Invite code required")
            return
        }
        val request = com.ekkus.silentdisco.core.protocol.ControlMessage.JoinRequest(
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
        simulateApprovalAndPlayback(session.id)
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
        )
    }

    fun retryJoin() {
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
        val base = SystemClock.elapsedRealtime()
        repeat(6) { index ->
            syncEstimator.observe(
                ClockSyncSample(
                    t1 = base + index * 20L,
                    t2 = base + index * 20L + 12L,
                    t3 = base + index * 20L + 16L,
                    t4 = base + index * 20L + 24L,
                ),
            )
        }
        val syncState = syncEstimator.snapshot().copy(
            resyncCount = _uiState.value.listenerSyncState.resyncCount + 1,
        )
        _uiState.value = _uiState.value.copy(listenerSyncState = syncState)
        diagnosticsStore.updateListener {
            it.copy(
                hostOffsetMs = syncState.offsetMs,
                rttMs = syncState.rttMs,
                jitterMs = syncState.jitterMs,
                resyncCount = syncState.resyncCount,
            )
        }
        refreshListenerDiagnostics()
    }

    private fun simulateApprovalAndPlayback(sessionId: String) {
        viewModelScope.launch {
            delay(500)
            _uiState.value = _uiState.value.copy(
                listenerState = ListenerLifecycleState.APPROVED,
                connectionProgress = _uiState.value.connectionProgress.copy(
                    currentState = ListenerLifecycleState.APPROVED,
                    approved = true,
                ),
            )
            delay(400)
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
            delay(300)
            _uiState.value = _uiState.value.copy(
                listenerState = ListenerLifecycleState.PLAYING,
                listenerPlaybackState = PlaybackState.PLAYING,
                connectionProgress = _uiState.value.connectionProgress.copy(
                    currentState = ListenerLifecycleState.PLAYING,
                    playing = true,
                ),
            )
            diagnosticsStore.updateListener {
                it.copy(
                    sessionId = sessionId,
                    bufferDepthMs = 420,
                    playbackState = PlaybackState.PLAYING,
                    lastPacketSequence = 19,
                )
            }
            refreshListenerDiagnostics()
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
                    it.joinState == JoinApprovalState.REQUESTED || it.joinState == JoinApprovalState.APPROVED
                },
                streamState = streamState,
            )
        }
        _uiState.value = _uiState.value.copy(hostDiagnostics = diagnosticsStore.hostDiagnostics.value)
    }

    private fun refreshListenerDiagnostics() {
        _uiState.value = _uiState.value.copy(listenerDiagnostics = diagnosticsStore.listenerDiagnostics.value)
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
