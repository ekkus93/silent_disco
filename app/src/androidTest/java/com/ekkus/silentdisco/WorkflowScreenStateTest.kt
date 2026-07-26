package com.ekkus.silentdisco

import androidx.compose.ui.test.assertDoesNotExist
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.StorageInitializationState
import com.ekkus.silentdisco.app.TuningSettings
import com.ekkus.silentdisco.core.model.AppRole
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.JoinRequest
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.feature.diagnostics.ConnectionHelpScreen
import com.ekkus.silentdisco.feature.diagnostics.DiagnosticsScreen
import com.ekkus.silentdisco.feature.host.HostDashboardScreen
import com.ekkus.silentdisco.feature.listener.ListenerPlaybackV2Screen
import com.ekkus.silentdisco.feature.listener.NearbySessionsScreen
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import org.junit.Rule
import org.junit.Test

class WorkflowScreenStateTest {
    @get:Rule
    val composeRule = createComposeRule()

    @Test
    fun hostDashboardShowsRequestProgressAndDisablesDuplicateActions() {
        val request = JoinRequest(
            requestId = "request-1",
            sessionId = "session-1",
            listenerId = "listener-1",
            listenerName = "Phillip’s phone",
            inviteCode = null,
            requestedAtMs = 0L,
        )
        composeRule.setContent {
            SilentDiscoTheme {
                HostDashboardScreen(
                    uiState = AppUiState(
                        storageState = StorageInitializationState.READY,
                        hostState = HostLifecycleState.READY,
                        pendingJoinRequests = listOf(request),
                    ),
                    onBackRequest = {},
                    onInvite = {},
                    onApproval = { _, _ -> },
                    onRemoveListener = {},
                    onPlayPause = {},
                    onStop = {},
                    onEndSessionRequest = {},
                    onOpenConnectionHelp = {},
                    onAddDemoJoinRequest = {},
                )
            }
        }

        composeRule.onNodeWithText("Always allow").performClick()
        composeRule.onNodeWithText("Remembering this phone, then sending approval…").assertIsDisplayed()
        composeRule.onNodeWithText("Approve once").assertIsNotEnabled()
        composeRule.onNodeWithText("Reject").assertIsNotEnabled()
    }

    @Test
    fun discoveryShowsPermissionScanningEmptyAndResultStates() {
        composeRule.setContent {
            SilentDiscoTheme {
                NearbySessionsScreen(
                    uiState = AppUiState(storageState = StorageInitializationState.READY),
                    permissionRequired = true,
                    onBack = {},
                    onRequestPermission = {},
                    onRefresh = {},
                    onSelectSession = {},
                )
            }
        }
        composeRule.onNodeWithTag("nearby-permission-required").assertIsDisplayed()

        composeRule.setContent {
            SilentDiscoTheme {
                NearbySessionsScreen(
                    uiState = AppUiState(
                        storageState = StorageInitializationState.READY,
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
        composeRule.onNodeWithTag("nearby-scanning").assertIsDisplayed()

        val session = SessionInfo(
            id = "session-1",
            name = "Kitchen Disco",
            hostDeviceName = "Host phone",
            approvalMode = ApprovalMode.MANUAL,
            inviteCodeRequired = false,
        )
        composeRule.setContent {
            SilentDiscoTheme {
                NearbySessionsScreen(
                    uiState = AppUiState(
                        storageState = StorageInitializationState.READY,
                        discoveredSessions = listOf(session),
                    ),
                    permissionRequired = false,
                    onBack = {},
                    onRequestPermission = {},
                    onRefresh = {},
                    onSelectSession = {},
                )
            }
        }
        composeRule.onNodeWithTag("nearby-results").assertIsDisplayed()
        composeRule.onNodeWithText("Approval required").assertIsDisplayed()
    }

    @Test
    fun healthyPlaybackHidesRecoveryWhileDesyncShowsIt() {
        val healthy = AppUiState(
            storageState = StorageInitializationState.READY,
            listenerState = ListenerLifecycleState.PLAYING,
            listenerPlaybackState = PlaybackState.PLAYING,
            listenerSyncState = AppUiState().listenerSyncState.copy(confidence = SyncQualityBadge.GOOD),
        )
        composeRule.setContent {
            SilentDiscoTheme {
                ListenerPlaybackV2Screen(
                    uiState = healthy,
                    onBackRequest = {},
                    onVolumeChanged = {},
                    onFixConnection = {},
                    onLeaveRequest = {},
                )
            }
        }
        composeRule.onNodeWithText("Playing in sync").assertIsDisplayed()
        composeRule.onNodeWithTag("listener-fix-connection").assertDoesNotExist()

        composeRule.setContent {
            SilentDiscoTheme {
                ListenerPlaybackV2Screen(
                    uiState = healthy.copy(listenerState = ListenerLifecycleState.DESYNCED),
                    onBackRequest = {},
                    onVolumeChanged = {},
                    onFixConnection = {},
                    onLeaveRequest = {},
                )
            }
        }
        composeRule.onNodeWithText("Audio is out of sync").assertIsDisplayed()
        composeRule.onNodeWithTag("listener-fix-connection").assertIsDisplayed()
    }

    @Test
    fun connectionHelpShowsOnlyValidRecoveryActions() {
        composeRule.setContent {
            SilentDiscoTheme {
                ConnectionHelpScreen(
                    uiState = AppUiState(
                        storageState = StorageInitializationState.READY,
                        selectedRole = AppRole.LISTENER,
                        listenerState = ListenerLifecycleState.DISCONNECTED,
                        listenerPlaybackState = PlaybackState.STOPPED,
                    ),
                    onBack = {},
                    onResynchronize = {},
                    onReconnect = {},
                    onShareSupportReport = {},
                    onAdvancedDiagnostics = {},
                )
            }
        }

        composeRule.onNodeWithTag("connection-help-reconnect").assertIsEnabled()
        composeRule.onNodeWithTag("connection-help-resync").assertDoesNotExist()
        composeRule.onNodeWithText("Advanced diagnostics").assertIsDisplayed()
    }

    @Test
    fun advancedDiagnosticsKeepsExpertTuningDisabledUntilEnabled() {
        composeRule.setContent {
            SilentDiscoTheme {
                DiagnosticsScreen(
                    uiState = AppUiState(
                        storageState = StorageInitializationState.READY,
                        selectedRole = AppRole.LISTENER,
                        tuningSettings = TuningSettings(),
                    ),
                    onBack = {},
                    onManualResync = {},
                    onAdjustTuning = { _, _ -> },
                    onShare = {},
                )
            }
        }

        composeRule.onNodeWithTag("expert-tuning-toggle").performClick()
        composeRule.onNodeWithText("Changing these values can make synchronization worse.").assertIsDisplayed()
        composeRule.onNodeWithText("−", useUnmergedTree = true).assertIsNotEnabled()
        composeRule.onNodeWithTag("enable-expert-tuning").performClick()
        composeRule.onNodeWithText("−", useUnmergedTree = true).assertIsEnabled()
    }
}
