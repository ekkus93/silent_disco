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
import com.ekkus.silentdisco.core.audio.DecodedAudioChunk
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
import com.ekkus.silentdisco.core.model.TrustState
import com.ekkus.silentdisco.core.permissions.PermissionCatalogue
import com.ekkus.silentdisco.core.permissions.AppPermission
import com.ekkus.silentdisco.core.permissions.PermissionState
import com.ekkus.silentdisco.core.rust.RustStoredTuningSettings
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

internal fun MainViewModel.scanForSessionsImpl() {
    if (!requirePersistenceReady("scan for sessions")) return
    logger.i("listener.scan", "Scanning for nearby sessions")
    scanJob?.cancel()
    _uiState.value = _uiState.value.copy(lastMessage = "Scanning for nearby sessions…", lastError = null)
    ensureRustListenerCore().startDiscovery()

    scanJob = viewModelScope.launch {
        val scanWindowMs = _uiState.value.tuningSettings.normalized().scanWindowMs
        delay(scanWindowMs)
        refreshDiscoveredSessions()
    }
}

internal fun MainViewModel.requestJoinImpl() {
    if (!requirePersistenceReady("join a session")) return
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
    val inviteCode = _uiState.value.connectionProgress.inviteCode.ifBlank { null }
    if (session.inviteCodeRequired && inviteCode == null) {
        _uiState.value = _uiState.value.copy(lastError = "Invite code required")
        return
    }
    logger.i("listener.join", "Join request created for ${session.id}")
    _uiState.value = _uiState.value.copy(lastMessage = "Connecting to host", lastError = null)
    val shouldSimulate = BuildConfig.DEBUG && session.id.startsWith(DEMO_SESSION_ID_PREFIX)
    if (shouldSimulate) {
        val shouldReject = session.inviteCodeRequired && inviteCode != "1234"
        simulateApprovalAndPlayback(session.id, shouldReject)
        return
    }
    ensureRustListenerCore().submitJoin(inviteCode)
}
