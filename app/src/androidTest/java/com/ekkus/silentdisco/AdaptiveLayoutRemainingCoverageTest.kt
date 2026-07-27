package com.ekkus.silentdisco

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasScrollAction
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNode
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToIndex
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.ConnectionProgressState
import com.ekkus.silentdisco.app.StorageInitializationState
import com.ekkus.silentdisco.app.TuningSettings
import com.ekkus.silentdisco.core.model.AppRole
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.JoinRequest
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.feature.diagnostics.DiagnosticsScreen
import com.ekkus.silentdisco.feature.host.HostDashboardScreen
import com.ekkus.silentdisco.feature.listener.SessionJoinScreen
import com.ekkus.silentdisco.feature.startup.StartupGateScreen
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
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

        composeRule.onNode(hasScrollAction()).performScrollToIndex(4)
        composeRule.onNodeWithText("Always allow").assertIsDisplayed()
        composeRule.onNodeWithText("Reject").assertIsDisplayed()
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

        composeRule.onNodeWithTag("session-problem-primary").performScrollTo().assertIsDisplayed()
        composeRule.onNodeWithTag("session-problem-secondary").performScrollTo().assertIsDisplayed()
        composeRule.onNodeWithText("Cancel").performScrollTo().assertIsDisplayed()
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
