package com.ekkus.silentdisco

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToIndex
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeUp
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.ConnectionProgressState
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
import com.ekkus.silentdisco.feature.diagnostics.DiagnosticsScreen
import com.ekkus.silentdisco.feature.host.HostDashboardScreen
import com.ekkus.silentdisco.feature.host.HostQueueManagementScreen
import com.ekkus.silentdisco.feature.listener.SessionJoinScreen
import com.ekkus.silentdisco.feature.startup.StartupGateScreen
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import org.junit.Rule
import org.junit.Test

class AdaptiveLayoutRemainingCoverageTest {
    @get:Rule
    val composeRule = createComposeRule()

    private val session = SessionInfo(
        id = "session-adaptive-remaining",
        name = "Warehouse Silent Disco",
        hostDeviceName = "Host phone",
        approvalMode = ApprovalMode.INVITE_CODE,
        inviteCodeRequired = true,
    )

    @Test
    fun hostDashboardApprovalActionsRemainReachableInSmallLargeTextWindow() {
        val request = JoinRequest(
            requestId = "request-adaptive",
            sessionId = session.id,
            listenerId = "listener-adaptive",
            listenerName = "Listener with a long accessible display name",
            inviteCode = null,
            requestedAtMs = 0L,
        )
        composeRule.setContent {
            RemainingAdaptiveSurface(width = 320.dp, height = 480.dp, fontScale = 2f) {
                HostDashboardScreen(
                    uiState = readyState().copy(
                        selectedRole = AppRole.HOST,
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

        repeat(5) {
            composeRule.onNodeWithTag("host-dashboard-list").performTouchInput { swipeUp() }
        }
        composeRule.onNodeWithText("Always allow").performScrollTo().assertIsDisplayed()
        composeRule.onNodeWithText("Reject").performScrollTo().assertIsDisplayed()
    }

    @Test
    fun liveQueueEditControlsStayVisibleAndUsableInSmallLargeTextWindow() {
        var addRequests = 0
        var removedEntry: String? = null
        var reorderedIds: List<String>? = null
        composeRule.setContent {
            RemainingAdaptiveSurface(width = 320.dp, height = 480.dp, fontScale = 2f) {
                HostQueueManagementScreen(
                    uiState = readyState().copy(
                        hostPlaybackState = PlaybackState.PLAYING,
                        hostQueue = listOf(
                            HostQueueItem("current", "source-1", "Current song", null, null),
                            HostQueueItem("second", "source-2", "Second song", null, null),
                            HostQueueItem("third", "source-3", "Third song", null, null),
                        ),
                    ),
                    onBack = {},
                    onAddSongs = { addRequests += 1 },
                    onReorderQueue = { reorderedIds = it },
                    onRemoveQueueEntry = { removedEntry = it },
                )
            }
        }

        // Unlike the growing collection, Add songs must stay pinned and visible.
        composeRule.onNodeWithTag("host-queue-add-songs").assertIsDisplayed().performClick()
        composeRule.onNodeWithTag("host-queue-upcoming-list").performScrollToIndex(0)

        listOf("host-queue-move-down-second", "host-queue-remove-second").forEach { tag ->
            val node = composeRule.onNodeWithTag(tag).assertIsDisplayed()
            val bounds = node.fetchSemanticsNode().boundsInRoot
            assertWithMessage("$tag left edge").that(bounds.left).isAtLeast(0f)
            assertWithMessage("$tag right edge").that(bounds.right).isAtMost(320f)
        }
        composeRule.onNodeWithTag("host-queue-move-down-second").performClick()
        composeRule.onNodeWithTag("host-queue-remove-second").performClick()
        composeRule.runOnIdle {
            assertThat(addRequests).isEqualTo(1)
            assertThat(removedEntry).isEqualTo("second")
            assertThat(reorderedIds)
                .containsExactly("current", "third", "second")
                .inOrder()
        }
    }

    @Test
    fun sessionJoinRecoveryActionsRemainReachableInSmallLargeTextWindow() {
        composeRule.setContent {
            RemainingAdaptiveSurface(width = 320.dp, height = 480.dp, fontScale = 2f) {
                SessionJoinScreen(
                    uiState = readyState().copy(
                        selectedRole = AppRole.LISTENER,
                        selectedSession = session,
                        discoveredSessions = listOf(session),
                        listenerState = ListenerLifecycleState.ERROR,
                        connectionProgress = ConnectionProgressState(
                            currentState = ListenerLifecycleState.ERROR,
                            discovered = true,
                            requested = true,
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

        composeRule.onNodeWithTag("session-problem-primary").assertIsDisplayed()
        composeRule.onNodeWithTag("session-problem-secondary").assertIsDisplayed()
        composeRule.onNodeWithText("Cancel").assertIsDisplayed()
        composeRule.onNodeWithTag("client-workflow-step-connect").assertIsDisplayed()
    }

    @Test
    fun advancedDiagnosticsActionsRemainReachableInLandscapeWindow() {
        composeRule.setContent {
            RemainingAdaptiveSurface(width = 640.dp, height = 360.dp) {
                DiagnosticsScreen(
                    uiState = readyState().copy(
                        selectedRole = AppRole.LISTENER,
                        selectedSession = session,
                        listenerState = ListenerLifecycleState.DESYNCED,
                        tuningSettings = TuningSettings(syncCadenceMs = 2_250L),
                        lastError = "Synchronization needs attention",
                    ),
                    onBack = {},
                    onManualResync = {},
                    onAdjustTuning = { _, _ -> },
                    onShare = {},
                )
            }
        }

        composeRule.onNodeWithTag("advanced-persistent-problem").performScrollTo().assertIsDisplayed()
        composeRule.onNodeWithTag("advanced-resynchronize").performScrollTo().assertIsDisplayed()
        composeRule.onNodeWithTag("advanced-share-report").performScrollTo().assertIsDisplayed()
    }

    @Test
    fun callbackCaptureControlsFollowRecordingAndFinalizingStatesInCompactLandscape() {
        var captureStatus by mutableStateOf("Capture off")
        var startRequests = 0
        var stopRequests = 0
        composeRule.setContent {
            RemainingAdaptiveSurface(width = 640.dp, height = 360.dp, fontScale = 1.5f) {
                DiagnosticsScreen(
                    uiState = readyState().copy(selectedRole = AppRole.LISTENER),
                    onBack = {},
                    onManualResync = {},
                    onAdjustTuning = { _, _ -> },
                    onShare = {},
                    callbackCaptureStatus = captureStatus,
                    onStartCallbackCapture = {
                        startRequests += 1
                        captureStatus = "Recording callback PCM"
                    },
                    onStopCallbackCapture = {
                        stopRequests += 1
                        captureStatus = "Finalizing capture"
                    },
                )
            }
        }

        composeRule.onNodeWithText("Playback output").performScrollTo().performClick()
        composeRule.onNodeWithTag("start-callback-capture")
            .performScrollTo().assertIsDisplayed().assertIsEnabled().performClick()
        composeRule.onNodeWithTag("stop-callback-capture")
            .performScrollTo().assertIsDisplayed().assertIsEnabled().performClick()
        composeRule.onNodeWithTag("start-callback-capture")
            .performScrollTo().assertIsDisplayed().assertIsNotEnabled()
        composeRule.runOnIdle {
            assertThat(startRequests).isEqualTo(1)
            assertThat(stopRequests).isEqualTo(1)
            captureStatus = "Saved 480 callback frames"
        }
        composeRule.onNodeWithTag("start-callback-capture")
            .performScrollTo().assertIsDisplayed().assertIsEnabled()
    }

    @Test
    fun recoverableStartupActionsRemainReachableInSmallLargeTextWindow() {
        composeRule.setContent {
            RemainingAdaptiveSurface(width = 320.dp, height = 480.dp, fontScale = 2f) {
                StartupGateScreen(
                    uiState = AppUiState(
                        storageState = StorageInitializationState.RECOVERABLE_FAILURE,
                        storageError = "database is temporarily busy",
                    ),
                    onRetry = {},
                    onShareSupportReport = {},
                )
            }
        }

        composeRule.onNodeWithTag("startup-retry").performScrollTo().assertIsDisplayed()
        composeRule.onNodeWithTag("startup-share-support").performScrollTo().assertIsDisplayed()
    }

    @Test
    fun fatalStartupSupportActionRemainsReachableInSmallLargeTextWindow() {
        composeRule.setContent {
            RemainingAdaptiveSurface(width = 320.dp, height = 480.dp, fontScale = 2f) {
                StartupGateScreen(
                    uiState = AppUiState(
                        storageState = StorageInitializationState.FATAL_FAILURE,
                        storageError = "migration checksum mismatch",
                    ),
                    onRetry = {},
                    onShareSupportReport = {},
                )
            }
        }

        composeRule.onNodeWithTag("startup-fatal").assertIsDisplayed()
        composeRule.onNodeWithTag("startup-share-support").performScrollTo().assertIsDisplayed()
    }

    private fun readyState(): AppUiState = AppUiState(
        storageState = StorageInitializationState.READY,
    )
}

@Composable
private fun RemainingAdaptiveSurface(
    width: Dp,
    height: Dp,
    fontScale: Float = 1f,
    content: @Composable () -> Unit,
) {
    CompositionLocalProvider(
        LocalDensity provides Density(density = 1f, fontScale = fontScale),
    ) {
        SilentDiscoTheme {
            Box(modifier = Modifier.size(width = width, height = height)) {
                content()
            }
        }
    }
}
