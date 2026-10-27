package com.ekkus.silentdisco

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextReplacement
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.ConnectionProgressState
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.feature.listener.SessionJoinScreen
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import com.google.common.truth.Truth.assertThat
import org.junit.Rule
import org.junit.Test

/** The active join status and recovery action must not disappear below a compact viewport. */
class SessionJoinPinnedStatusTest {
    @get:Rule val compose = createComposeRule()

    private val session = SessionInfo(
        id = "pinned-status-session",
        name = "Warehouse Disco",
        hostDeviceName = "Host phone",
        approvalMode = ApprovalMode.MANUAL,
        inviteCodeRequired = true,
    )

    @Test
    fun compactLargeTextStagesStayVisibleAndSurviveSavedStateRestoration() {
        var state by mutableStateOf(
            AppUiState(
                selectedSession = session,
                listenerState = ListenerLifecycleState.AWAITING_APPROVAL,
                connectionProgress = ConnectionProgressState(
                    discovered = true,
                    requested = true,
                    inviteCode = "4826",
                ),
            ),
        )
        var cancelled = 0
        val restoration = StateRestorationTester(compose)
        restoration.setContent {
            CompositionLocalProvider(LocalDensity provides Density(1f, 1.75f)) {
                SilentDiscoTheme {
                    Box(Modifier.size(320.dp, 480.dp)) {
                        SessionJoinScreen(
                            uiState = state,
                            onInviteCodeChanged = {},
                            onJoin = {},
                            onCancel = { cancelled++ },
                            onRetry = {},
                            onReturnToSessions = {},
                            onOpenSettings = {},
                            onShareSupportReport = {},
                        )
                    }
                }
            }
        }
        val status = compose.onNodeWithTag("session-join-active-status")
        status.assertIsDisplayed().assertTextContains("Waiting for host approval")
        assertThat(status.fetchSemanticsNode().config[SemanticsProperties.LiveRegion])
            .isEqualTo(LiveRegionMode.Polite)
        val cancel = compose.onNodeWithTag("session-join-cancel").assertIsDisplayed()
        assertThat(status.fetchSemanticsNode().boundsInRoot.bottom)
            .isLessThan(cancel.fetchSemanticsNode().boundsInRoot.top)

        compose.runOnIdle {
            state = state.copy(
                listenerState = ListenerLifecycleState.APPROVED,
                connectionProgress = state.connectionProgress.copy(approved = true),
            )
        }
        status.assertIsDisplayed().assertTextContains("Waiting for the host to start playback")
        compose.runOnIdle {
            state = state.copy(
                listenerState = ListenerLifecycleState.SYNCING_CLOCK,
                connectionProgress = state.connectionProgress.copy(synced = true),
            )
        }
        status.assertIsDisplayed().assertTextContains("Getting audio in sync")
        restoration.emulateSavedInstanceStateRestore()
        status.assertIsDisplayed().assertTextContains("Getting audio in sync")
        cancel.assertIsDisplayed().performClick()
        compose.runOnIdle { assertThat(cancelled).isEqualTo(1) }
    }

    @Test
    fun rejectedInviteCodeAnnouncesPinnedErrorAndRestoresEditableRecovery() {
        var state by mutableStateOf(
            AppUiState(
                selectedSession = session,
                listenerState = ListenerLifecycleState.ERROR,
                connectionProgress = ConnectionProgressState(inviteCode = "1111"),
                lastError = "Incorrect invite code",
            ),
        )
        var retries = 0
        val restoration = StateRestorationTester(compose)
        restoration.setContent {
            CompositionLocalProvider(LocalDensity provides Density(1f, 1.5f)) {
                SilentDiscoTheme {
                    Box(Modifier.size(320.dp, 480.dp)) {
                        SessionJoinScreen(
                            uiState = state,
                            onInviteCodeChanged = { code ->
                                state = state.copy(
                                    connectionProgress = state.connectionProgress.copy(inviteCode = code),
                                )
                            },
                            onJoin = { retries++ },
                            onCancel = {},
                            onRetry = {},
                            onReturnToSessions = {},
                            onOpenSettings = {},
                            onShareSupportReport = {},
                        )
                    }
                }
            }
        }
        val status = compose.onNodeWithTag("session-join-active-status")
        status.assertIsDisplayed().assertTextContains("The invite code was not accepted")
        assertThat(status.fetchSemanticsNode().config[SemanticsProperties.LiveRegion])
            .isEqualTo(LiveRegionMode.Assertive)
        compose.onNodeWithTag("session-problem-primary").assertIsDisplayed().performClick()
        assertThat(compose.onAllNodesWithTag("session-join-active-status").fetchSemanticsNodes())
            .isEmpty()
        // The edit action must bring the scrollable field into view
        // automatically, even at 320x480 dp with enlarged system text.
        compose.onNodeWithTag("session-join-code")
            .assertIsDisplayed()
            .performTextReplacement("4826")
        compose.runOnIdle {
            assertThat(state.connectionProgress.inviteCode).isEqualTo("4826")
        }
        restoration.emulateSavedInstanceStateRestore()
        compose.onNodeWithTag("session-join-code").assertIsDisplayed()
        compose.onNodeWithTag("session-request-join").assertIsDisplayed().performClick()
        compose.runOnIdle {
            assertThat(retries).isEqualTo(1)
            assertThat(state.connectionProgress.inviteCode).isEqualTo("4826")
        }
        status.assertIsDisplayed().assertTextContains("The invite code was not accepted")
    }
}
