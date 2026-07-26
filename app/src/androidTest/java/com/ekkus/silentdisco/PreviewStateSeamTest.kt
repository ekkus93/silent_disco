package com.ekkus.silentdisco

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.HostFormState
import com.ekkus.silentdisco.app.JoinApprovalAction
import com.ekkus.silentdisco.app.StorageInitializationState
import com.ekkus.silentdisco.app.TuningSettings
import com.ekkus.silentdisco.core.model.AppRole
import com.ekkus.silentdisco.core.model.HostDiagnosticsSnapshot
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.JoinApprovalState
import com.ekkus.silentdisco.core.model.ListenerInfo
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.model.TrustState
import com.ekkus.silentdisco.feature.diagnostics.DiagnosticsScreen
import com.ekkus.silentdisco.feature.host.HostDashboardScreen
import com.ekkus.silentdisco.feature.host.HostDashboardTab
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import org.junit.Rule
import org.junit.Test

class PreviewStateSeamTest {
    @get:Rule
    val composeRule = createComposeRule()

    @Test
    fun connectedTabCanBeRenderedAsInitialState() {
        val listener = listener(
            id = "listener-connected",
            name = "Alex's phone",
            connectionState = TransportConnectionState.CONNECTED,
            syncQuality = SyncQualityBadge.GOOD,
            listenerState = ListenerLifecycleState.PLAYING,
        )
        composeRule.setContent {
            SilentDiscoTheme {
                HostDashboardScreen(
                    uiState = hostState(listener),
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

        composeRule.onNodeWithText("Alex's phone").assertIsDisplayed()
        composeRule.onNodeWithText("No one is waiting to join.").assertDoesNotExist()
    }

    @Test
    fun needsAttentionTabCanBeRenderedAsInitialState() {
        val listener = listener(
            id = "listener-attention",
            name = "Riley's phone",
            connectionState = TransportConnectionState.RETRYING,
            syncQuality = SyncQualityBadge.POOR,
            listenerState = ListenerLifecycleState.DESYNCED,
        )
        composeRule.setContent {
            SilentDiscoTheme {
                HostDashboardScreen(
                    uiState = hostState(listener).copy(lastError = "One listener is out of sync"),
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

        composeRule.onNodeWithText("Riley's phone").assertIsDisplayed()
        composeRule.onNodeWithText("No listeners need attention.").assertDoesNotExist()
    }

    @Test
    fun expertControlsCanBeRenderedExpandedAndEnabled() {
        composeRule.setContent {
            SilentDiscoTheme {
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

        composeRule.onNodeWithText("Changing these values can make synchronization worse.").assertIsDisplayed()
        composeRule.onNodeWithTag("enable-expert-tuning").assertDoesNotExist()
        composeRule.onAllNodesWithText("−", useUnmergedTree = true)[0].assertIsEnabled()
    }

    private fun hostState(listener: ListenerInfo): AppUiState = AppUiState(
        storageState = StorageInitializationState.READY,
        selectedRole = AppRole.HOST,
        hostForm = HostFormState(sessionName = "Warehouse Mix"),
        hostState = HostLifecycleState.STREAMING,
        hostPlaybackState = PlaybackState.PLAYING,
        approvedListeners = listOf(listener),
        hostDiagnostics = HostDiagnosticsSnapshot(
            sessionId = "preview-seam-session",
            listenerCount = 1,
            connectedListenerCount = 1,
            desyncedListenerCount = if (listener.listenerState == ListenerLifecycleState.DESYNCED) 1 else 0,
            streamState = PlaybackState.PLAYING,
        ),
    )

    private fun listener(
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
}
