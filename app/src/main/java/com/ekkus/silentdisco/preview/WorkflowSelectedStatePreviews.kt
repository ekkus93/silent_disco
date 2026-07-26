package com.ekkus.silentdisco.preview

import android.net.Uri
import androidx.compose.runtime.Composable
import androidx.compose.ui.tooling.preview.Preview
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.HostFormState
import com.ekkus.silentdisco.app.JoinApprovalAction
import com.ekkus.silentdisco.app.StorageInitializationState
import com.ekkus.silentdisco.app.TuningSettings
import com.ekkus.silentdisco.core.model.AppRole
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.HostDiagnosticsSnapshot
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.JoinApprovalState
import com.ekkus.silentdisco.core.model.ListenerInfo
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.model.TrustState
import com.ekkus.silentdisco.feature.diagnostics.DiagnosticsScreen
import com.ekkus.silentdisco.feature.host.HostDashboardScreen
import com.ekkus.silentdisco.feature.host.HostDashboardTab
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme

private const val SELECTED_STATE_BACKGROUND = 0xFF0F0E1A

private val selectedStateAudio = SelectedAudioFile(
    uri = Uri.parse("content://preview/music/night-mix.flac"),
    displayName = "Night mix.flac",
    mimeType = "audio/flac",
    sizeBytes = 24_000_000L,
)

private val selectedStateHostForm = HostFormState(
    sessionName = "Warehouse Mix",
    approvalMode = ApprovalMode.MANUAL,
    selectedAudio = selectedStateAudio,
)

private fun selectedStateListener(
    id: String,
    name: String,
    connectionState: TransportConnectionState,
    syncQuality: SyncQualityBadge,
    listenerState: ListenerLifecycleState,
): ListenerInfo = ListenerInfo(
    deviceId = id,
    displayName = name,
    joinState = JoinApprovalState.APPROVED,
    trustState = TrustState.SESSION_ONLY,
    connectionState = connectionState,
    listenerState = listenerState,
    syncQuality = syncQuality,
)

@Composable
private fun SelectedStateSurface(content: @Composable () -> Unit) {
    SilentDiscoTheme(content)
}

@Preview(
    name = "Host dashboard - Connected tab selected",
    showBackground = true,
    backgroundColor = SELECTED_STATE_BACKGROUND,
)
@Composable
private fun HostDashboardConnectedTabPreview() {
    val listener = selectedStateListener(
        id = "listener-connected",
        name = "Alex's phone",
        connectionState = TransportConnectionState.CONNECTED,
        syncQuality = SyncQualityBadge.GOOD,
        listenerState = ListenerLifecycleState.PLAYING,
    )
    SelectedStateSurface {
        HostDashboardScreen(
            uiState = AppUiState(
                storageState = StorageInitializationState.READY,
                selectedRole = AppRole.HOST,
                hostForm = selectedStateHostForm,
                hostState = HostLifecycleState.STREAMING,
                hostPlaybackState = PlaybackState.PLAYING,
                approvedListeners = listOf(listener),
                hostDiagnostics = HostDiagnosticsSnapshot(
                    sessionId = "preview-selected-connected",
                    listenerCount = 1,
                    connectedListenerCount = 1,
                    streamState = PlaybackState.PLAYING,
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
            initialTab = HostDashboardTab.CONNECTED,
        )
    }
}

@Preview(
    name = "Host dashboard - Needs attention tab selected",
    showBackground = true,
    backgroundColor = SELECTED_STATE_BACKGROUND,
)
@Composable
private fun HostDashboardNeedsAttentionTabPreview() {
    val listener = selectedStateListener(
        id = "listener-attention",
        name = "Riley's phone",
        connectionState = TransportConnectionState.RETRYING,
        syncQuality = SyncQualityBadge.POOR,
        listenerState = ListenerLifecycleState.DESYNCED,
    )
    SelectedStateSurface {
        HostDashboardScreen(
            uiState = AppUiState(
                storageState = StorageInitializationState.READY,
                selectedRole = AppRole.HOST,
                hostForm = selectedStateHostForm,
                hostState = HostLifecycleState.STREAMING,
                hostPlaybackState = PlaybackState.PLAYING,
                approvedListeners = listOf(listener),
                hostDiagnostics = HostDiagnosticsSnapshot(
                    sessionId = "preview-selected-attention",
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
            initialTab = HostDashboardTab.NEEDS_ATTENTION,
        )
    }
}

@Preview(
    name = "Advanced diagnostics - expert controls enabled",
    showBackground = true,
    backgroundColor = SELECTED_STATE_BACKGROUND,
)
@Composable
private fun AdvancedDiagnosticsExpertEnabledPreview() {
    SelectedStateSurface {
        DiagnosticsScreen(
            uiState = AppUiState(
                storageState = StorageInitializationState.READY,
                selectedRole = AppRole.LISTENER,
                listenerState = ListenerLifecycleState.PLAYING,
                tuningSettings = TuningSettings(syncCadenceMs = 2_250L),
            ),
            onBack = {},
            onManualResync = {},
            onAdjustTuning = { _, _ -> },
            onShare = {},
            initialExpertExpanded = true,
            initialExpertEnabled = true,
        )
    }
}
