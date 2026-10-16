package com.ekkus.silentdisco

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToNode
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.HostQueueItem
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
import com.ekkus.silentdisco.feature.host.HostQueueManagementScreen
import com.ekkus.silentdisco.feature.listener.ListenerPlaybackV2Screen
import com.ekkus.silentdisco.feature.listener.NearbySessionsScreen
import com.ekkus.silentdisco.feature.listener.SessionJoinScreen
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import com.google.common.truth.Truth.assertThat
import org.junit.Rule
import org.junit.Test

class WorkflowScreenStateTest {
    @get:Rule
    val composeRule = createComposeRule()

    @Test
    fun hostDashboardPinsSongTransportAndQueueActions() {
        var addRequested = false
        var queueRequested = false
        composeRule.setContent {
            SilentDiscoTheme {
                HostDashboardScreen(
                    uiState = AppUiState(
                        storageState = StorageInitializationState.READY,
                        hostState = HostLifecycleState.STREAMING,
                        hostPlaybackState = PlaybackState.PLAYING,
                        hostPlaybackPositionMs = 15_000L,
                        hostQueue = listOf(HostQueueItem("current", "source", "Pinned live track", null, 60_000L)),
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
                    onAddSongs = { addRequested = true },
                    onManageQueue = { queueRequested = true },
                )
            }
        }

        composeRule.onNodeWithTag("host-current-song").assertIsDisplayed()
        composeRule.onNodeWithTag("host-playback-progress").assertIsDisplayed()
        composeRule.onNodeWithText("0:15 / 1:00").assertIsDisplayed()
        composeRule.onNodeWithText("Pause").assertIsDisplayed()
        composeRule.onNodeWithText("Stop").assertIsDisplayed()
        composeRule.onNodeWithContentDescription("Add songs").assertIsDisplayed().performClick()
        composeRule.onNodeWithContentDescription("Manage queue").assertIsDisplayed().performClick()
        composeRule.runOnIdle {
            assertThat(addRequested).isTrue()
            assertThat(queueRequested).isTrue()
        }
    }

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

        composeRule.onNodeWithTag("host-dashboard-list")
            .performScrollToNode(hasText("Always allow"))
        composeRule.onNodeWithText("Always allow").performClick()
        composeRule.waitForIdle()
        composeRule.onNodeWithTag("approval-progress-request-1")
            .performScrollTo()
            .assertIsDisplayed()
        composeRule.onNodeWithText("Approve once").performScrollTo().assertIsNotEnabled()
        composeRule.onNodeWithText("Reject").performScrollTo().assertIsNotEnabled()
    }

    @Test
    fun liveHostQueueLocksCurrentAndAllowsUpcomingReorder() {
        var reordered: List<String>? = null
        var removed: String? = null
        composeRule.setContent {
            SilentDiscoTheme {
                HostQueueManagementScreen(
                    uiState = AppUiState(
                        hostPlaybackState = PlaybackState.PLAYING,
                        hostQueue = listOf(
                            HostQueueItem("current", "source-1", "Current song", null, null),
                            HostQueueItem("upcoming-1", "source-2", "Second song", null, null),
                            HostQueueItem("upcoming-2", "source-3", "Third song", null, null),
                        ),
                    ),
                    onBack = {},
                    onAddSongs = {},
                    onReorderQueue = { reordered = it },
                    onRemoveQueueEntry = { removed = it },
                )
            }
        }

        composeRule.onNodeWithTag("host-queue-current").assertIsDisplayed()
        composeRule.onNodeWithTag("host-queue-add-songs").assertIsDisplayed()
        composeRule.onNodeWithContentDescription("Move Second song up").assertIsNotEnabled()
        composeRule.onNodeWithContentDescription("Move Second song down").assertIsEnabled()
        composeRule.onNodeWithContentDescription("Remove Second song").assertIsEnabled()
        composeRule.onNodeWithTag("host-queue-move-up-upcoming-1").assertIsNotEnabled()
        composeRule.onNodeWithTag("host-queue-move-down-upcoming-1").performClick()
        composeRule.runOnIdle {
            assertThat(reordered).containsExactly("current", "upcoming-2", "upcoming-1").inOrder()
            assertThat(removed).isNull()
        }
        composeRule.onNodeWithTag("host-queue-remove-upcoming-1").performClick()
        composeRule.runOnIdle { assertThat(removed).isEqualTo("upcoming-1") }
    }

    @Test
    fun emptyHostQueueKeepsAddActionPinnedAndInvokable() {
        var addRequested = false
        composeRule.setContent {
            SilentDiscoTheme {
                HostQueueManagementScreen(
                    uiState = AppUiState(storageState = StorageInitializationState.READY),
                    onBack = {},
                    onAddSongs = { addRequested = true },
                    onReorderQueue = {},
                    onRemoveQueueEntry = {},
                )
            }
        }

        composeRule.onNodeWithTag("host-queue-add-songs").assertIsDisplayed().performClick()
        composeRule.runOnIdle { assertThat(addRequested).isTrue() }
        composeRule.onNodeWithText("No song is playing. Add a song, then press Play.")
            .assertIsDisplayed()
    }

    @Test
    fun failedCurrentSongCanBeRemovedFromQueueManagement() {
        var removed: String? = null
        composeRule.setContent {
            SilentDiscoTheme {
                HostQueueManagementScreen(
                    uiState = AppUiState(
                        hostPlaybackState = PlaybackState.ERROR,
                        hostQueue = listOf(
                            HostQueueItem("failed-entry", "failed-source", "Unreadable song", null, null),
                        ),
                    ),
                    onBack = {},
                    onAddSongs = {},
                    onReorderQueue = {},
                    onRemoveQueueEntry = { removed = it },
                )
            }
        }

        composeRule.onNodeWithText("Song needs attention").assertIsDisplayed()
        composeRule.onNodeWithText("Playback stopped. Retry or remove this song.").assertIsDisplayed()
        composeRule.onNodeWithTag("host-queue-remove-failed-current").performClick()
        composeRule.runOnIdle { assertThat(removed).isEqualTo("failed-entry") }
    }

    @Test
    fun discoveryShowsPermissionRequiredState() {
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
        composeRule.onNodeWithTag("client-workflow-step-find").assertIsDisplayed()
    }

    @Test
    fun discoveryShowsScanningState() {
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
        composeRule.onNodeWithTag("client-workflow-step-find").assertIsDisplayed()
    }

    @Test
    fun selectedSessionShowsConnectStageAndPinnedJoinAction() {
        val session = SessionInfo(
            id = "session-connect-stage",
            name = "Garden Disco",
            hostDeviceName = "Host phone",
            approvalMode = ApprovalMode.MANUAL,
            inviteCodeRequired = false,
        )
        composeRule.setContent {
            SilentDiscoTheme {
                SessionJoinScreen(
                    uiState = AppUiState(
                        storageState = StorageInitializationState.READY,
                        selectedSession = session,
                        listenerState = ListenerLifecycleState.SESSION_SELECTED,
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

        composeRule.onNodeWithTag("client-workflow-step-connect").assertIsDisplayed()
        composeRule.onNodeWithTag("session-request-join").assertIsDisplayed()
    }

    @Test
    fun connectManuallyActionOnlyAppearsWhenCallbackProvided() {
        composeRule.setContent {
            SilentDiscoTheme {
                NearbySessionsScreen(
                    uiState = AppUiState(storageState = StorageInitializationState.READY),
                    permissionRequired = false,
                    onBack = {},
                    onRequestPermission = {},
                    onRefresh = {},
                    onSelectSession = {},
                    onConnectManually = {},
                )
            }
        }

        composeRule.onNodeWithTag("nearby-connect-manually").assertIsDisplayed()
    }

    @Test
    fun discoveryKeepsQrScanActionInItsFixedHeader() {
        var scanned = false
        composeRule.setContent {
            SilentDiscoTheme {
                NearbySessionsScreen(
                    uiState = AppUiState(storageState = StorageInitializationState.READY),
                    permissionRequired = false,
                    onBack = {},
                    onRequestPermission = {},
                    onRefresh = {},
                    onSelectSession = {},
                    onScanQr = { scanned = true },
                )
            }
        }

        composeRule.onNodeWithTag("nearby-scan-qr").assertIsDisplayed().performClick()
        composeRule.runOnIdle { assertThat(scanned).isTrue() }
    }

    @Test
    fun discoveryShowsPlainLanguageResultBadge() {
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
    fun healthyPlaybackHidesRecoveryAction() {
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
    }

    @Test
    fun listenerShowsRustAnnouncedCurrentTrackInsteadOfHostLocalSelection() {
        composeRule.setContent {
            SilentDiscoTheme {
                ListenerPlaybackV2Screen(
                    uiState = AppUiState(
                        storageState = StorageInitializationState.READY,
                        listenerState = ListenerLifecycleState.PLAYING,
                        listenerPlaybackState = PlaybackState.PLAYING,
                        listenerCurrentTrackName = "Shared current track",
                    ),
                    onBackRequest = {},
                    onVolumeChanged = {},
                    onFixConnection = {},
                    onLeaveRequest = {},
                )
            }
        }

        composeRule.onNodeWithTag("listener-current-track").assertIsDisplayed()
        composeRule.onNodeWithTag("client-workflow-step-listen").assertIsDisplayed()
        composeRule.onNodeWithText("Shared current track").assertIsDisplayed()
        composeRule.onNodeWithText("Waiting for track information").assertDoesNotExist()
    }

    @Test
    fun desynchronizedPlaybackShowsRecoveryAction() {
        composeRule.setContent {
            SilentDiscoTheme {
                ListenerPlaybackV2Screen(
                    uiState = AppUiState(
                        storageState = StorageInitializationState.READY,
                        listenerState = ListenerLifecycleState.DESYNCED,
                        listenerPlaybackState = PlaybackState.PLAYING,
                    ),
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
        composeRule.onNodeWithText("Changing these values can make synchronization worse.")
            .assertIsDisplayed()
        composeRule.onAllNodesWithText("−")[0].performScrollTo().assertIsNotEnabled()
        composeRule.onNodeWithTag("enable-expert-tuning").performScrollTo().performClick()
        composeRule.onAllNodesWithText("−")[0].performScrollTo().assertIsEnabled()
    }
}
