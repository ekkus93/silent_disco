package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.AppRole
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.HostDiagnosticsSnapshot
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.JoinApprovalState
import com.ekkus.silentdisco.core.model.JoinRequest
import com.ekkus.silentdisco.core.model.ListenerDiagnosticsSnapshot
import com.ekkus.silentdisco.core.model.ListenerInfo
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.model.SyncState
import com.ekkus.silentdisco.core.permissions.PermissionState

data class HostFormState(
    val sessionName: String = "Silent Disco Session",
    val approvalMode: ApprovalMode = ApprovalMode.MANUAL,
    val inviteCode: String = "",
    val rememberApprovedDevices: Boolean = false,
    val selectedAudio: SelectedAudioFile? = null,
)

data class ConnectionProgressState(
    val currentState: ListenerLifecycleState = ListenerLifecycleState.IDLE,
    val discovered: Boolean = false,
    val requested: Boolean = false,
    val approved: Boolean = false,
    val connected: Boolean = false,
    val synced: Boolean = false,
    val playing: Boolean = false,
    val inviteCode: String = "",
)

data class AppUiState(
    val selectedRole: AppRole? = null,
    val permissions: List<PermissionState> = emptyList(),
    val hostForm: HostFormState = HostFormState(),
    val hostState: HostLifecycleState = HostLifecycleState.IDLE,
    val listenerState: ListenerLifecycleState = ListenerLifecycleState.IDLE,
    val connectionProgress: ConnectionProgressState = ConnectionProgressState(),
    val discoveredSessions: List<SessionInfo> = emptyList(),
    val selectedSession: SessionInfo? = null,
    val pendingJoinRequests: List<JoinRequest> = emptyList(),
    val approvedListeners: List<ListenerInfo> = emptyList(),
    val hostPlaybackState: PlaybackState = PlaybackState.STOPPED,
    val listenerPlaybackState: PlaybackState = PlaybackState.STOPPED,
    val listenerSyncState: SyncState = SyncState(),
    val hostDiagnostics: HostDiagnosticsSnapshot = HostDiagnosticsSnapshot(),
    val listenerDiagnostics: ListenerDiagnosticsSnapshot = ListenerDiagnosticsSnapshot(),
    val localVolume: Float = 1.0f,
    val lastMessage: String? = null,
    val lastError: String? = null,
)

fun AppUiState.permissionSummary(): String {
    if (permissions.isEmpty()) return "Permissions not checked"
    val granted = permissions.count { it.granted }
    return "$granted/${permissions.size} permissions granted"
}

fun AppUiState.syncSummary(): String = when (listenerSyncState.confidence) {
    SyncQualityBadge.EXCELLENT -> "Excellent sync"
    SyncQualityBadge.GOOD -> "Good sync"
    SyncQualityBadge.FAIR -> "Fair sync"
    SyncQualityBadge.POOR -> "Poor sync"
    SyncQualityBadge.UNKNOWN -> "Sync unknown"
}

fun AppUiState.connectionQualitySummary(): String = when {
    lastError != null -> "Connection trouble"
    listenerState == ListenerLifecycleState.DESYNCED -> "Needs resync"
    listenerDiagnostics.underrunCount > 0 || listenerDiagnostics.lateDropCount > 0 -> "Recovering"
    listenerSyncState.confidence == SyncQualityBadge.EXCELLENT ||
        listenerSyncState.confidence == SyncQualityBadge.GOOD -> "Stable"
    else -> "Monitoring"
}

fun AppUiState.listenerStateLabel(): String = when (listenerState) {
    ListenerLifecycleState.IDLE -> "Idle"
    ListenerLifecycleState.SCANNING -> "Scanning"
    ListenerLifecycleState.SESSION_SELECTED -> "Session selected"
    ListenerLifecycleState.JOIN_REQUESTED -> "Join requested"
    ListenerLifecycleState.AWAITING_APPROVAL -> "Awaiting approval"
    ListenerLifecycleState.APPROVED -> "Approved"
    ListenerLifecycleState.CONNECTING -> "Connecting"
    ListenerLifecycleState.SYNCING_CLOCK -> "Syncing clock"
    ListenerLifecycleState.BUFFERING -> "Buffering"
    ListenerLifecycleState.PLAYING -> "Playing"
    ListenerLifecycleState.RECONNECTING -> "Reconnecting"
    ListenerLifecycleState.DESYNCED -> "Desynced"
    ListenerLifecycleState.DISCONNECTED -> "Disconnected"
    ListenerLifecycleState.ERROR -> "Error"
}

fun JoinRequest.toListenerInfo(): ListenerInfo = ListenerInfo(
    deviceId = listenerId,
    displayName = listenerName,
    joinState = JoinApprovalState.REQUESTED,
    trustState = com.ekkus.silentdisco.core.model.TrustState.SESSION_ONLY,
    connectionState = com.ekkus.silentdisco.core.model.TransportConnectionState.CONNECTING,
    listenerState = ListenerLifecycleState.CONNECTING,
    syncQuality = SyncQualityBadge.UNKNOWN,
)
