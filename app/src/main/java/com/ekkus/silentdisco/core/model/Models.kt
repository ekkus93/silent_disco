package com.ekkus.silentdisco.core.model

import android.net.Uri

enum class AppRole {
    HOST,
    LISTENER,
}

enum class ApprovalMode {
    MANUAL,
    TRUSTED_DEVICES_PLACEHOLDER,
    INVITE_CODE,
}

enum class TrustState {
    SESSION_ONLY,
    TRUSTED_PLACEHOLDER,
}

enum class HostLifecycleState {
    IDLE,
    CREATING_SESSION,
    ADVERTISING,
    WAITING_FOR_LISTENERS,
    READY,
    STREAMING,
    PAUSED,
    ENDING_SESSION,
    ERROR,
}

enum class ListenerLifecycleState {
    IDLE,
    SCANNING,
    SESSION_SELECTED,
    JOIN_REQUESTED,
    AWAITING_APPROVAL,
    APPROVED,
    CONNECTING,
    SYNCING_CLOCK,
    BUFFERING,
    PLAYING,
    RECONNECTING,
    DESYNCED,
    DISCONNECTED,
    ERROR,
}

enum class JoinApprovalState {
    DISCOVERED,
    REQUESTED,
    APPROVED,
    REJECTED,
    CANCELLED,
}

enum class TransportConnectionState {
    IDLE,
    DISCOVERING,
    ADVERTISING,
    CONNECTING,
    CONNECTED,
    RETRYING,
    DISCONNECTED,
    FAILED,
}

enum class PlaybackState {
    STOPPED,
    BUFFERING,
    READY,
    PLAYING,
    PAUSED,
    UNDERRUN,
    ERROR,
}

enum class SyncQualityBadge {
    UNKNOWN,
    POOR,
    FAIR,
    GOOD,
    EXCELLENT,
}

data class SessionInfo(
    val id: String,
    val name: String,
    val hostDeviceName: String,
    val approvalMode: ApprovalMode,
    val inviteCodeRequired: Boolean,
)

data class HostInfo(
    val deviceId: String,
    val displayName: String,
    val sessionInfo: SessionInfo?,
    val state: HostLifecycleState,
)

data class ListenerInfo(
    val deviceId: String,
    val displayName: String,
    val joinState: JoinApprovalState,
    val trustState: TrustState,
    val connectionState: TransportConnectionState,
    val listenerState: ListenerLifecycleState,
    val syncQuality: SyncQualityBadge,
)

data class JoinRequest(
    val requestId: String,
    val sessionId: String,
    val listenerId: String,
    val listenerName: String,
    val inviteCode: String?,
    val requestedAtMs: Long,
)

data class ApprovalDecision(
    val requestId: String,
    val approved: Boolean,
    val reason: String? = null,
    val trustedForFuture: Boolean = false,
)

data class SelectedAudioFile(
    val uri: Uri,
    val displayName: String,
    val mimeType: String?,
    val sizeBytes: Long?,
)

data class SyncState(
    val offsetMs: Double = 0.0,
    val rttMs: Double = 0.0,
    val jitterMs: Double = 0.0,
    val skewPpm: Double = 0.0,
    val confidence: SyncQualityBadge = SyncQualityBadge.UNKNOWN,
    val resyncCount: Int = 0,
)

data class HostDiagnosticsSnapshot(
    val sessionId: String = "",
    val listenerCount: Int = 0,
    val pendingJoinCount: Int = 0,
    val connectedListenerCount: Int = 0,
    val packetSendCount: Long = 0,
    val packetSendRatePerSecond: Double = 0.0,
    val streamState: PlaybackState = PlaybackState.STOPPED,
    val lastError: String? = null,
)

data class ListenerDiagnosticsSnapshot(
    val sessionId: String = "",
    val hostOffsetMs: Double = 0.0,
    val rttMs: Double = 0.0,
    val jitterMs: Double = 0.0,
    val bufferDepthMs: Long = 0,
    val packetLossCount: Int = 0,
    val lateDropCount: Int = 0,
    val underrunCount: Int = 0,
    val playbackState: PlaybackState = PlaybackState.STOPPED,
    val reconnectCount: Int = 0,
    val resyncCount: Int = 0,
    val lastPacketSequence: Long? = null,
    val lastError: String? = null,
)
