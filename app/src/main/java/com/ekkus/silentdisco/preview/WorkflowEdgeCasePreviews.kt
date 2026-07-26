package com.ekkus.silentdisco.preview

import android.net.Uri
import androidx.compose.runtime.Composable
import androidx.compose.ui.tooling.preview.Preview
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.ConnectionProgressState
import com.ekkus.silentdisco.app.HostFormState
import com.ekkus.silentdisco.app.JoinApprovalAction
import com.ekkus.silentdisco.app.StorageInitializationState
import com.ekkus.silentdisco.core.model.AppRole
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.HostDiagnosticsSnapshot
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.JoinApprovalState
import com.ekkus.silentdisco.core.model.ListenerInfo
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.model.SyncState
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.model.TrustState
import com.ekkus.silentdisco.feature.host.HostDashboardScreen
import com.ekkus.silentdisco.feature.listener.ListenerPlaybackV2Screen
import com.ekkus.silentdisco.feature.listener.SessionJoinScreen
import com.ekkus.silentdisco.feature.startup.StartupGateScreen
import com.ekkus.silentdisco.ui.components.ConfirmationSheet
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme

private const val EDGE_PREVIEW_BACKGROUND = 0xFF0F0E1A

private val edgeAudio = SelectedAudioFile(
    uri = Uri.parse("content://preview/music/night-mix.flac"),
    displayName = "Night mix.flac",
    mimeType = "audio/flac",
    sizeBytes = 24_000_000L,
)

private val edgeSession = SessionInfo(
    id = "preview-edge-session",
    name = "Warehouse Mix",
    hostDeviceName = "Jordan's phone",
    approvalMode = ApprovalMode.MANUAL,
    inviteCodeRequired = false,
)

private val edgeHostForm = HostFormState(
    sessionName = edgeSession.name,
    approvalMode = ApprovalMode.MANUAL,
    selectedAudio = edgeAudio,
)

private fun edgeReadyState(): AppUiState = AppUiState(
    storageState = StorageInitializationState.READY,
)

private fun connectedListener(
    id: String,
    name: String,
    syncQuality: SyncQualityBadge,
    listenerState: ListenerLifecycleState = ListenerLifecycleState.PLAYING,
): ListenerInfo = ListenerInfo(
    deviceId = id,
    displayName = name,
    joinState = JoinApprovalState.APPROVED,
    trustState = TrustState.SESSION_ONLY,
    connectionState = TransportConnectionState.CONNECTED,
    listenerState = listenerState,
    syncQuality = syncQuality,
)

@Composable
private fun EdgePreviewSurface(content: @Composable () -> Unit) {
    SilentDiscoTheme(content)
}

@Preview(
    name = "Startup - fatal failure",
    showBackground = true,
    backgroundColor = EDGE_PREVIEW_BACKGROUND,
)
@Composable
private fun StartupFatalFailurePreview() {
    EdgePreviewSurface {
        StartupGateScreen(
            uiState = AppUiState(
                storageState = StorageInitializationState.FATAL_FAILURE,
                storageError = "The migration checksum does not match this app version.",
            ),
            onRetry = {},
            onShareSupportReport = {},
        )
    }
}

@Preview(
    name = "Host dashboard - connected listeners",
    showBackground = true,
    backgroundColor = EDGE_PREVIEW_BACKGROUND,
)
@Composable
private fun HostDashboardConnectedPreview() {
    val listeners = listOf(
        connectedListener("listener-a", "Alex's phone", SyncQualityBadge.EXCELLENT),
        connectedListener("listener-b", "Sam's phone", SyncQualityBadge.GOOD),
    )
    EdgePreviewSurface {
        HostDashboardScreen(
            uiState = edgeReadyState().copy(
                selectedRole = AppRole.HOST,
                hostForm = edgeHostForm,
                hostState = HostLifecycleState.STREAMING,
                hostPlaybackState = PlaybackState.PLAYING,
                approvedListeners = listeners,
                hostDiagnostics = HostDiagnosticsSnapshot(
                    sessionId = edgeSession.id,
                    listenerCount = listeners.size,
                    connectedListenerCount = listeners.size,
                    streamState = PlaybackState.PLAYING,
                    packetSendCount = 8_420L,
                    packetSendRatePerSecond = 50.0,
                ),
            ),
            onBackRequest = {},
            onInvite = {},
            onApproval = { _, _: JoinApprovalAction -> },
            onRemoveListener = {},
            onPlayPause = {},
            onStop = {},
            onEndSessionRequest = {},
            onOpenConnectionHelp = {},
            onAddDemoJoinRequest = {},
        )
    }
}

@Preview(
    name = "Host dashboard - listener needs attention",
    showBackground = true,
    backgroundColor = EDGE_PREVIEW_BACKGROUND,
)
@Composable
private fun HostDashboardAttentionPreview() {
    val listeners = listOf(
        connectedListener(
            id = "listener-desynced",
            name = "Riley's phone",
            syncQuality = SyncQualityBadge.POOR,
            listenerState = ListenerLifecycleState.DESYNCED,
        ),
    )
    EdgePreviewSurface {
        HostDashboardScreen(
            uiState = edgeReadyState().copy(
                selectedRole = AppRole.HOST,
                hostForm = edgeHostForm,
                hostState = HostLifecycleState.STREAMING,
                hostPlaybackState = PlaybackState.PLAYING,
                approvedListeners = listeners,
                hostDiagnostics = HostDiagnosticsSnapshot(
                    sessionId = edgeSession.id,
                    listenerCount = 1,
                    connectedListenerCount = 1,
                    desyncedListenerCount = 1,
                    streamState = PlaybackState.PLAYING,
                    lastError = "One listener is out of sync",
                ),
                lastError = "One listener is out of sync",
            ),
            onBackRequest = {},
            onInvite = {},
            onApproval = { _, _: JoinApprovalAction -> },
            onRemoveListener = {},
            onPlayPause = {},
            onStop = {},
            onEndSessionRequest = {},
            onOpenConnectionHelp = {},
            onAddDemoJoinRequest = {},
        )
    }
}

@Preview(
    name = "Session join - connection failure",
    showBackground = true,
    backgroundColor = EDGE_PREVIEW_BACKGROUND,
)
@Composable
private fun SessionJoinConnectionFailurePreview() {
    EdgePreviewSurface {
        SessionJoinScreen(
            uiState = edgeReadyState().copy(
                selectedRole = AppRole.LISTENER,
                discoveredSessions = listOf(edgeSession),
                selectedSession = edgeSession,
                listenerState = ListenerLifecycleState.ERROR,
                listenerPlaybackState = PlaybackState.ERROR,
                connectionProgress = ConnectionProgressState(
                    currentState = ListenerLifecycleState.CONNECTING,
                    discovered = true,
                    requested = true,
                    approved = true,
                    connected = false,
                ),
                lastError = "The host could not be reached",
            ),
            onInviteCodeChanged = {},
            onJoin = {},
            onCancel = {},
            onRetry = {},
            onReturnToSessions = {},
            onOpenSettings = {},
            onShareSupportReport = {},
        )
    }
}

@Preview(
    name = "Now playing - reconnecting",
    showBackground = true,
    backgroundColor = EDGE_PREVIEW_BACKGROUND,
)
@Composable
private fun ListenerPlaybackReconnectingPreview() {
    EdgePreviewSurface {
        ListenerPlaybackV2Screen(
            uiState = edgeReadyState().copy(
                selectedRole = AppRole.LISTENER,
                hostForm = edgeHostForm,
                selectedSession = edgeSession,
                listenerState = ListenerLifecycleState.RECONNECTING,
                listenerPlaybackState = PlaybackState.BUFFERING,
                listenerSyncState = SyncState(confidence = SyncQualityBadge.FAIR),
                connectionProgress = ConnectionProgressState(
                    currentState = ListenerLifecycleState.RECONNECTING,
                    discovered = true,
                    requested = true,
                    approved = true,
                    connected = false,
                    synced = true,
                    buffered = false,
                    playing = false,
                ),
                lastError = "The connection was interrupted",
                localVolume = 0.58f,
            ),
            onBackRequest = {},
            onVolumeChanged = {},
            onFixConnection = {},
            onLeaveRequest = {},
        )
    }
}

@Preview(
    name = "Confirmation - end session",
    showBackground = true,
    backgroundColor = EDGE_PREVIEW_BACKGROUND,
)
@Composable
private fun EndSessionConfirmationPreview() {
    EdgePreviewSurface {
        ConfirmationSheet(
            visible = true,
            title = "End this session?",
            detail = "Playback will stop for 3 connected listeners.",
            safeActionLabel = "Keep hosting",
            destructiveActionLabel = "End session",
            onDismiss = {},
            onConfirm = {},
            testTag = "preview-end-session-confirmation",
        )
    }
}

@Preview(
    name = "Confirmation - leave session",
    showBackground = true,
    backgroundColor = EDGE_PREVIEW_BACKGROUND,
)
@Composable
private fun LeaveSessionConfirmationPreview() {
    EdgePreviewSurface {
        ConfirmationSheet(
            visible = true,
            title = "Leave this session?",
            detail = "Audio playback on this phone will stop.",
            safeActionLabel = "Stay",
            destructiveActionLabel = "Leave session",
            onDismiss = {},
            onConfirm = {},
            testTag = "preview-leave-session-confirmation",
        )
    }
}
