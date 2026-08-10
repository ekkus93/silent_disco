package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.AppRole
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.HostDiagnosticsSnapshot
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.JoinApprovalState
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.model.TrustState
import com.ekkus.silentdisco.core.model.JoinRequest
import com.ekkus.silentdisco.core.model.ListenerDiagnosticsSnapshot
import com.ekkus.silentdisco.core.model.ListenerInfo
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.ManualConnectUiState
import com.ekkus.silentdisco.core.model.ManualEndpointFormState
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
    val scanWindowMs: Long = 3_000L,
) {
    fun normalized(): TuningSettings =
        copy(scanWindowMs = scanWindowMs.coerceIn(1_000L, 10_000L))
}

enum class TuningField {
    SyncSampleWindow,
    SyncCadenceMs,
    StartupBufferMs,
    LatePacketThresholdMs,
    HardResyncThresholdMs,
    SyncDriftThresholdMs,
    ScanWindowMs,
    ResetDefaults,
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
    val buffered: Boolean = false,
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
    val manualEndpointForm: ManualEndpointFormState = ManualEndpointFormState(),
    val manualConnectState: ManualConnectUiState = ManualConnectUiState.Idle,
    val pendingJoinRequests: List<JoinRequest> = emptyList(),
    val approvedListeners: List<ListenerInfo> = emptyList(),
    val hostPlaybackState: PlaybackState = PlaybackState.STOPPED,
    val listenerPlaybackState: PlaybackState = PlaybackState.STOPPED,
    val listenerSyncState: SyncState = SyncState(),
    val tuningSettings: TuningSettings = TuningSettings(),
    val storageState: StorageInitializationState = StorageInitializationState.INITIALIZING,
    val storageError: String? = null,
    val hostDiagnostics: HostDiagnosticsSnapshot = HostDiagnosticsSnapshot(),
    val listenerDiagnostics: ListenerDiagnosticsSnapshot = ListenerDiagnosticsSnapshot(),
    val localVolume: Float = 1.0f,
    val isScanning: Boolean = false,
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

fun JoinRequest.toListenerInfo(
    trustState: TrustState = TrustState.SESSION_ONLY,
): ListenerInfo = ListenerInfo(
    deviceId = listenerId,
    displayName = listenerName,
    joinState = JoinApprovalState.REQUESTED,
    trustState = trustState,
    connectionState = TransportConnectionState.CONNECTING,
    listenerState = ListenerLifecycleState.CONNECTING,
    syncQuality = SyncQualityBadge.UNKNOWN,
)

fun TuningSettings.adjust(field: TuningField, direction: Int): TuningSettings {
    if (field == TuningField.ResetDefaults) return TuningSettings()
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
        TuningField.ScanWindowMs -> copy(scanWindowMs = (scanWindowMs + (step * 500L)).coerceIn(1_000L, 10_000L))
        TuningField.ResetDefaults -> error("ResetDefaults is handled before incremental tuning")
    }
    val lateThreshold = updated.latePacketThresholdMs.coerceAtMost(updated.hardResyncThresholdMs - 20L)
    return updated.copy(
        latePacketThresholdMs = lateThreshold.coerceAtLeast(10L),
        hardResyncThresholdMs = updated.hardResyncThresholdMs.coerceAtLeast(lateThreshold + 20L),
    )
}

fun TuningSettings.summary(): String =
    "samples=$syncSampleWindow, cadence=${syncCadenceMs}ms, startup=${startupBufferMs}ms, late=${latePacketThresholdMs}ms, resync=${hardResyncThresholdMs}ms, drift=${"%.1f".format(syncDriftThresholdMs)}ms"

fun HostLifecycleState.label(): String = when (this) {
    HostLifecycleState.IDLE -> "Idle"
    HostLifecycleState.CREATING_SESSION -> "Creating session…"
    HostLifecycleState.ADVERTISING -> "Advertising…"
    HostLifecycleState.WAITING_FOR_LISTENERS -> "Waiting for listeners"
    HostLifecycleState.READY -> "Ready"
    HostLifecycleState.STREAMING -> "Streaming"
    HostLifecycleState.PAUSED -> "Paused"
    HostLifecycleState.ENDING_SESSION -> "Ending session…"
    HostLifecycleState.ERROR -> "Error"
}

fun PlaybackState.label(): String = when (this) {
    PlaybackState.STOPPED -> "Stopped"
    PlaybackState.BUFFERING -> "Buffering…"
    PlaybackState.READY -> "Ready"
    PlaybackState.PLAYING -> "Playing"
    PlaybackState.PAUSED -> "Paused"
    PlaybackState.UNDERRUN -> "Underrun"
    PlaybackState.ERROR -> "Error"
}

fun ApprovalMode.label(): String = when (this) {
    ApprovalMode.MANUAL -> "Manual approval"
    ApprovalMode.TRUSTED_DEVICES_PLACEHOLDER -> "Trusted devices"
    ApprovalMode.INVITE_CODE -> "Invite code"
}

fun JoinApprovalState.label(): String = when (this) {
    JoinApprovalState.DISCOVERED -> "Discovered"
    JoinApprovalState.REQUESTED -> "Pending"
    JoinApprovalState.APPROVED -> "Approved"
    JoinApprovalState.REJECTED -> "Rejected"
    JoinApprovalState.CANCELLED -> "Cancelled"
}

fun TransportConnectionState.label(): String = when (this) {
    TransportConnectionState.IDLE -> "Idle"
    TransportConnectionState.DISCOVERING -> "Discovering…"
    TransportConnectionState.ADVERTISING -> "Advertising…"
    TransportConnectionState.CONNECTING -> "Connecting…"
    TransportConnectionState.CONNECTED -> "Connected"
    TransportConnectionState.RETRYING -> "Retrying…"
    TransportConnectionState.DISCONNECTED -> "Disconnected"
    TransportConnectionState.FAILED -> "Failed"
}

fun SyncQualityBadge.label(): String = when (this) {
    SyncQualityBadge.UNKNOWN -> "Unknown"
    SyncQualityBadge.POOR -> "Poor"
    SyncQualityBadge.FAIR -> "Fair"
    SyncQualityBadge.GOOD -> "Good"
    SyncQualityBadge.EXCELLENT -> "Excellent"
}

enum class StepState { Pending, Active, Done }

fun ConnectionProgressState.discoveredStep(): StepState =
    if (discovered) StepState.Done else StepState.Active

fun ConnectionProgressState.requestedStep(): StepState = when {
    requested -> StepState.Done
    discovered -> StepState.Active
    else -> StepState.Pending
}

fun ConnectionProgressState.approvedStep(): StepState = when {
    approved -> StepState.Done
    requested -> StepState.Active
    else -> StepState.Pending
}

fun ConnectionProgressState.connectedStep(): StepState = when {
    connected -> StepState.Done
    approved -> StepState.Active
    else -> StepState.Pending
}

fun ConnectionProgressState.syncedStep(): StepState = when {
    synced -> StepState.Done
    connected -> StepState.Active
    else -> StepState.Pending
}

fun ConnectionProgressState.bufferingStep(): StepState = when {
    buffered -> StepState.Done
    synced -> StepState.Active
    else -> StepState.Pending
}

fun ConnectionProgressState.playingStep(): StepState = when {
    playing -> StepState.Done
    buffered -> StepState.Active
    else -> StepState.Pending
}

fun AppUiState.isJoinInProgress(): Boolean = listenerState in setOf(
    ListenerLifecycleState.JOIN_REQUESTED,
    ListenerLifecycleState.AWAITING_APPROVAL,
    ListenerLifecycleState.APPROVED,
    ListenerLifecycleState.CONNECTING,
    ListenerLifecycleState.SYNCING_CLOCK,
    ListenerLifecycleState.BUFFERING,
    ListenerLifecycleState.PLAYING,
    ListenerLifecycleState.RECONNECTING,
    ListenerLifecycleState.DESYNCED,
)

fun AppUiState.canSelectSession(session: SessionInfo): Boolean =
    !isJoinInProgress() || selectedSession?.id == session.id

fun AppUiState.canManualResync(): Boolean =
    selectedSession != null && listenerState !in setOf(
        ListenerLifecycleState.IDLE,
        ListenerLifecycleState.SCANNING,
        ListenerLifecycleState.SESSION_SELECTED,
        ListenerLifecycleState.DISCONNECTED,
        ListenerLifecycleState.ERROR,
    )

/**
 * Single source of truth for the host dashboard's play/pause button
 * legality, so Compose renders it rather than re-deriving the same
 * condition inline (Block 21.2: button enablement must derive from a
 * canonical presentation helper, not be reconstructed per screen).
 */
fun AppUiState.canStartOrPauseHostPlayback(): Boolean =
    hostForm.selectedAudio != null && hostPlaybackState != PlaybackState.BUFFERING

/** Companion legality check for the host dashboard's stop button; see [canStartOrPauseHostPlayback]. */
fun AppUiState.canStopHostPlayback(): Boolean =
    hostPlaybackState !in setOf(PlaybackState.STOPPED, PlaybackState.ERROR)
