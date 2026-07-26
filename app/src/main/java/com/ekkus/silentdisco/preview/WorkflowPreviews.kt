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
import com.ekkus.silentdisco.core.model.JoinRequest
import com.ekkus.silentdisco.core.model.ListenerDiagnosticsSnapshot
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.model.SyncState
import com.ekkus.silentdisco.feature.diagnostics.ConnectionHelpScreen
import com.ekkus.silentdisco.feature.diagnostics.DiagnosticsScreen
import com.ekkus.silentdisco.feature.home.RoleFirstHomeScreen
import com.ekkus.silentdisco.feature.host.HostAccessSetupScreen
import com.ekkus.silentdisco.feature.host.HostDashboardScreen
import com.ekkus.silentdisco.feature.host.HostMusicSetupScreen
import com.ekkus.silentdisco.feature.listener.ListenerPlaybackV2Screen
import com.ekkus.silentdisco.feature.listener.NearbySessionsScreen
import com.ekkus.silentdisco.feature.listener.SessionJoinScreen
import com.ekkus.silentdisco.feature.settings.SettingsScreen
import com.ekkus.silentdisco.feature.startup.StartupGateScreen
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme

private const val PREVIEW_BACKGROUND = 0xFF0F0E1A

private val previewAudio = SelectedAudioFile(
    uri = Uri.parse("content://preview/music/rooftop-set.wav"),
    displayName = "Rooftop set.wav",
    mimeType = "audio/wav",
    sizeBytes = 18_500_000L,
)

private val manualSession = SessionInfo(
    id = "preview-session-manual",
    name = "Rooftop Silent Disco",
    hostDeviceName = "Morgan's phone",
    approvalMode = ApprovalMode.MANUAL,
    inviteCodeRequired = false,
)

private val inviteSession = SessionInfo(
    id = "preview-session-invite",
    name = "Back Patio Mix",
    hostDeviceName = "Casey's phone",
    approvalMode = ApprovalMode.INVITE_CODE,
    inviteCodeRequired = true,
)

private val completeHostForm = HostFormState(
    sessionName = "Rooftop Silent Disco",
    approvalMode = ApprovalMode.MANUAL,
    selectedAudio = previewAudio,
)

private val inviteHostForm = completeHostForm.copy(
    approvalMode = ApprovalMode.INVITE_CODE,
    inviteCode = "4821",
)

private fun readyState(): AppUiState = AppUiState(
    storageState = StorageInitializationState.READY,
)

private fun healthyListenerState(): AppUiState = readyState().copy(
    selectedRole = AppRole.LISTENER,
    hostForm = completeHostForm,
    selectedSession = manualSession,
    listenerState = ListenerLifecycleState.PLAYING,
    listenerPlaybackState = PlaybackState.PLAYING,
    listenerSyncState = SyncState(
        offsetMs = 2.1,
        rttMs = 18.0,
        jitterMs = 1.7,
        confidence = SyncQualityBadge.GOOD,
    ),
    listenerDiagnostics = ListenerDiagnosticsSnapshot(
        sessionId = manualSession.id,
        hostOffsetMs = 2.1,
        rttMs = 18.0,
        jitterMs = 1.7,
        bufferDepthMs = 420L,
        playbackState = PlaybackState.PLAYING,
        playbackPositionMs = 84_000L,
        metricsSummary = "connection stable",
    ),
    connectionProgress = ConnectionProgressState(
        currentState = ListenerLifecycleState.PLAYING,
        discovered = true,
        requested = true,
        approved = true,
        connected = true,
        synced = true,
        buffered = true,
        playing = true,
    ),
    localVolume = 0.72f,
)

@Composable
private fun PreviewSurface(content: @Composable () -> Unit) {
    SilentDiscoTheme(content)
}

@Preview(
    name = "Startup - loading",
    showBackground = true,
    backgroundColor = PREVIEW_BACKGROUND,
)
@Composable
private fun StartupLoadingPreview() {
    PreviewSurface {
        StartupGateScreen(
            uiState = AppUiState(storageState = StorageInitializationState.INITIALIZING),
            onRetry = {},
            onShareSupportReport = {},
        )
    }
}

@Preview(
    name = "Startup - recoverable failure",
    showBackground = true,
    backgroundColor = PREVIEW_BACKGROUND,
)
@Composable
private fun StartupRecoverableFailurePreview() {
    PreviewSurface {
        StartupGateScreen(
            uiState = AppUiState(
                storageState = StorageInitializationState.RECOVERABLE_FAILURE,
                storageError = "The local database is temporarily locked.",
            ),
            onRetry = {},
            onShareSupportReport = {},
        )
    }
}

@Preview(
    name = "Home - ready",
    showBackground = true,
    backgroundColor = PREVIEW_BACKGROUND,
)
@Composable
private fun HomeReadyPreview() {
    PreviewSurface {
        RoleFirstHomeScreen(
            uiState = readyState(),
            onHostClick = {},
            onJoinClick = {},
            onSettingsClick = {},
            onRetryStorage = {},
        )
    }
}

@Preview(
    name = "Host music - complete",
    showBackground = true,
    backgroundColor = PREVIEW_BACKGROUND,
)
@Composable
private fun HostMusicCompletePreview() {
    PreviewSurface {
        HostMusicSetupScreen(
            uiState = readyState().copy(hostForm = completeHostForm),
            onBack = {},
            onSessionNameChanged = {},
            onChooseAudio = {},
            onNext = {},
        )
    }
}

@Preview(
    name = "Host access - invite code",
    showBackground = true,
    backgroundColor = PREVIEW_BACKGROUND,
)
@Composable
private fun HostAccessInviteCodePreview() {
    PreviewSurface {
        HostAccessSetupScreen(
            uiState = readyState().copy(hostForm = inviteHostForm),
            onBack = {},
            onApprovalModeChanged = {},
            onInviteCodeChanged = {},
            onGenerateCode = {},
            onStartSession = {},
            onOpenSettings = {},
            onShareSupportReport = {},
        )
    }
}

@Preview(
    name = "Host dashboard - no listeners",
    showBackground = true,
    backgroundColor = PREVIEW_BACKGROUND,
)
@Composable
private fun HostDashboardEmptyPreview() {
    PreviewSurface {
        HostDashboardScreen(
            uiState = readyState().copy(
                selectedRole = AppRole.HOST,
                hostForm = completeHostForm,
                hostState = HostLifecycleState.READY,
                hostDiagnostics = HostDiagnosticsSnapshot(
                    sessionId = manualSession.id,
                    streamState = PlaybackState.STOPPED,
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
    name = "Host dashboard - pending request",
    showBackground = true,
    backgroundColor = PREVIEW_BACKGROUND,
)
@Composable
private fun HostDashboardPendingPreview() {
    val request = JoinRequest(
        requestId = "preview-request",
        sessionId = manualSession.id,
        listenerId = "preview-listener",
        listenerName = "Taylor's phone",
        inviteCode = null,
        requestedAtMs = 0L,
    )
    PreviewSurface {
        HostDashboardScreen(
            uiState = readyState().copy(
                selectedRole = AppRole.HOST,
                hostForm = completeHostForm,
                hostState = HostLifecycleState.READY,
                pendingJoinRequests = listOf(request),
                hostDiagnostics = HostDiagnosticsSnapshot(
                    sessionId = manualSession.id,
                    listenerCount = 1,
                    pendingJoinCount = 1,
                    streamState = PlaybackState.STOPPED,
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
    name = "Nearby sessions - scanning",
    showBackground = true,
    backgroundColor = PREVIEW_BACKGROUND,
)
@Composable
private fun NearbyScanningPreview() {
    PreviewSurface {
        NearbySessionsScreen(
            uiState = readyState().copy(
                listenerState = ListenerLifecycleState.SCANNING,
                isScanning = true,
            ),
            permissionRequired = false,
            onBack = {},
            onRequestPermission = {},
            onRefresh = {},
            onSelectSession = {},
        )
    }
}

@Preview(
    name = "Nearby sessions - empty",
    showBackground = true,
    backgroundColor = PREVIEW_BACKGROUND,
)
@Composable
private fun NearbyEmptyPreview() {
    PreviewSurface {
        NearbySessionsScreen(
            uiState = readyState(),
            permissionRequired = false,
            onBack = {},
            onRequestPermission = {},
            onRefresh = {},
            onSelectSession = {},
        )
    }
}

@Preview(
    name = "Nearby sessions - results",
    showBackground = true,
    backgroundColor = PREVIEW_BACKGROUND,
)
@Composable
private fun NearbyResultsPreview() {
    PreviewSurface {
        NearbySessionsScreen(
            uiState = readyState().copy(
                discoveredSessions = listOf(manualSession, inviteSession),
            ),
            permissionRequired = false,
            onBack = {},
            onRequestPermission = {},
            onRefresh = {},
            onSelectSession = {},
        )
    }
}

@Preview(
    name = "Session join - before request",
    showBackground = true,
    backgroundColor = PREVIEW_BACKGROUND,
)
@Composable
private fun SessionJoinBeforeRequestPreview() {
    PreviewSurface {
        SessionJoinScreen(
            uiState = readyState().copy(
                selectedRole = AppRole.LISTENER,
                discoveredSessions = listOf(inviteSession),
                selectedSession = inviteSession,
                listenerState = ListenerLifecycleState.SESSION_SELECTED,
                connectionProgress = ConnectionProgressState(
                    currentState = ListenerLifecycleState.SESSION_SELECTED,
                    discovered = true,
                    inviteCode = "4821",
                ),
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
    name = "Session join - waiting for approval",
    showBackground = true,
    backgroundColor = PREVIEW_BACKGROUND,
)
@Composable
private fun SessionJoinWaitingPreview() {
    PreviewSurface {
        SessionJoinScreen(
            uiState = readyState().copy(
                selectedRole = AppRole.LISTENER,
                discoveredSessions = listOf(manualSession),
                selectedSession = manualSession,
                listenerState = ListenerLifecycleState.AWAITING_APPROVAL,
                connectionProgress = ConnectionProgressState(
                    currentState = ListenerLifecycleState.AWAITING_APPROVAL,
                    discovered = true,
                    requested = true,
                    connected = true,
                ),
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
    name = "Session join - rejected invite code",
    showBackground = true,
    backgroundColor = PREVIEW_BACKGROUND,
)
@Composable
private fun SessionJoinRejectedPreview() {
    PreviewSurface {
        SessionJoinScreen(
            uiState = readyState().copy(
                selectedRole = AppRole.LISTENER,
                discoveredSessions = listOf(inviteSession),
                selectedSession = inviteSession,
                listenerState = ListenerLifecycleState.ERROR,
                connectionProgress = ConnectionProgressState(
                    currentState = ListenerLifecycleState.AWAITING_APPROVAL,
                    discovered = true,
                    requested = true,
                    connected = true,
                    inviteCode = "1111",
                ),
                lastError = "Incorrect invite code",
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
    name = "Now playing - healthy",
    showBackground = true,
    backgroundColor = PREVIEW_BACKGROUND,
)
@Composable
private fun ListenerPlaybackHealthyPreview() {
    PreviewSurface {
        ListenerPlaybackV2Screen(
            uiState = healthyListenerState(),
            onBackRequest = {},
            onVolumeChanged = {},
            onFixConnection = {},
            onLeaveRequest = {},
        )
    }
}

@Preview(
    name = "Now playing - buffering",
    showBackground = true,
    backgroundColor = PREVIEW_BACKGROUND,
)
@Composable
private fun ListenerPlaybackBufferingPreview() {
    PreviewSurface {
        ListenerPlaybackV2Screen(
            uiState = healthyListenerState().copy(
                listenerState = ListenerLifecycleState.BUFFERING,
                listenerPlaybackState = PlaybackState.BUFFERING,
                connectionProgress = healthyListenerState().connectionProgress.copy(
                    currentState = ListenerLifecycleState.BUFFERING,
                    buffered = false,
                    playing = false,
                ),
            ),
            onBackRequest = {},
            onVolumeChanged = {},
            onFixConnection = {},
            onLeaveRequest = {},
        )
    }
}

@Preview(
    name = "Now playing - out of sync",
    showBackground = true,
    backgroundColor = PREVIEW_BACKGROUND,
)
@Composable
private fun ListenerPlaybackDesyncedPreview() {
    PreviewSurface {
        ListenerPlaybackV2Screen(
            uiState = healthyListenerState().copy(
                listenerState = ListenerLifecycleState.DESYNCED,
                listenerSyncState = SyncState(
                    offsetMs = 73.0,
                    rttMs = 96.0,
                    jitterMs = 24.0,
                    confidence = SyncQualityBadge.POOR,
                ),
                lastError = "Audio drift exceeded the recovery threshold",
            ),
            onBackRequest = {},
            onVolumeChanged = {},
            onFixConnection = {},
            onLeaveRequest = {},
        )
    }
}

@Preview(
    name = "Connection help - healthy",
    showBackground = true,
    backgroundColor = PREVIEW_BACKGROUND,
)
@Composable
private fun ConnectionHelpHealthyPreview() {
    PreviewSurface {
        ConnectionHelpScreen(
            uiState = healthyListenerState(),
            onBack = {},
            onResynchronize = {},
            onReconnect = {},
            onShareSupportReport = {},
            onAdvancedDiagnostics = {},
        )
    }
}

@Preview(
    name = "Connection help - actionable",
    showBackground = true,
    backgroundColor = PREVIEW_BACKGROUND,
)
@Composable
private fun ConnectionHelpActionablePreview() {
    PreviewSurface {
        ConnectionHelpScreen(
            uiState = healthyListenerState().copy(
                listenerState = ListenerLifecycleState.DESYNCED,
                listenerPlaybackState = PlaybackState.PLAYING,
                listenerSyncState = SyncState(confidence = SyncQualityBadge.POOR),
                lastError = "Audio synchronization needs attention",
            ),
            onBack = {},
            onResynchronize = {},
            onReconnect = {},
            onShareSupportReport = {},
            onAdvancedDiagnostics = {},
        )
    }
}

@Preview(
    name = "Advanced diagnostics - listener",
    showBackground = true,
    backgroundColor = PREVIEW_BACKGROUND,
)
@Composable
private fun AdvancedDiagnosticsPreview() {
    PreviewSurface {
        DiagnosticsScreen(
            uiState = healthyListenerState(),
            onBack = {},
            onManualResync = {},
            onAdjustTuning = { _, _ -> },
            onShare = {},
        )
    }
}

@Preview(
    name = "Settings",
    showBackground = true,
    backgroundColor = PREVIEW_BACKGROUND,
)
@Composable
private fun SettingsPreview() {
    PreviewSurface {
        SettingsScreen(
            uiState = readyState(),
            trustedDeviceManagementAvailable = false,
            onBack = {},
            onOpenSystemSettings = {},
            onOpenTrustedDevices = {},
            onOpenAdvancedDiagnostics = {},
        )
    }
}
