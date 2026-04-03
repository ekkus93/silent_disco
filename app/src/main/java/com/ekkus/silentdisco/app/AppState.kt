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

data class TuningSettings(
    val syncSampleWindow: Int = 12,
    val syncCadenceMs: Long = 2_000,
    val startupBufferMs: Long = 400,
    val latePacketThresholdMs: Long = 40,
    val hardResyncThresholdMs: Long = 120,
    val syncDriftThresholdMs: Double = 18.0,
)

enum class TuningField {
    SyncSampleWindow,
    SyncCadenceMs,
    StartupBufferMs,
    LatePacketThresholdMs,
    HardResyncThresholdMs,
    SyncDriftThresholdMs,
}

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
    val tuningSettings: TuningSettings = TuningSettings(),
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

fun TuningSettings.adjust(field: TuningField, direction: Int): TuningSettings {
    val step = when {
        direction > 0 -> 1
        direction < 0 -> -1
        else -> 0
    }
    if (step == 0) return this
    val updated = when (field) {
        TuningField.SyncSampleWindow -> copy(syncSampleWindow = (syncSampleWindow + (step * 2)).coerceIn(4, 32))
        TuningField.SyncCadenceMs -> copy(syncCadenceMs = (syncCadenceMs + (step * 250L)).coerceIn(500L, 5_000L))
        TuningField.StartupBufferMs -> copy(startupBufferMs = (startupBufferMs + (step * 50L)).coerceIn(100L, 1_500L))
        TuningField.LatePacketThresholdMs -> copy(latePacketThresholdMs = (latePacketThresholdMs + (step * 5L)).coerceIn(10L, 250L))
        TuningField.HardResyncThresholdMs -> copy(hardResyncThresholdMs = (hardResyncThresholdMs + (step * 20L)).coerceIn(40L, 500L))
        TuningField.SyncDriftThresholdMs -> copy(syncDriftThresholdMs = (syncDriftThresholdMs + (step * 2.0)).coerceIn(4.0, 100.0))
    }
    val lateThreshold = updated.latePacketThresholdMs.coerceAtMost(updated.hardResyncThresholdMs - 20L)
    return updated.copy(
        latePacketThresholdMs = lateThreshold.coerceAtLeast(10L),
        hardResyncThresholdMs = updated.hardResyncThresholdMs.coerceAtLeast(lateThreshold + 20L),
    )
}

fun TuningSettings.summary(): String =
    "samples=$syncSampleWindow, cadence=${syncCadenceMs}ms, startup=${startupBufferMs}ms, late=${latePacketThresholdMs}ms, resync=${hardResyncThresholdMs}ms, drift=${"%.1f".format(syncDriftThresholdMs)}ms"
