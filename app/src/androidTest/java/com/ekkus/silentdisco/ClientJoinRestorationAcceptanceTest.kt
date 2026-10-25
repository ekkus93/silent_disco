package com.ekkus.silentdisco

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.assertDoesNotExist
import androidx.compose.ui.test.assertExists
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.ConnectionProgressState
import com.ekkus.silentdisco.core.model.AppRole
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.feature.listener.SessionJoinScreen
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import com.google.common.truth.Truth.assertThat
import org.junit.Rule
import org.junit.Test

/** Real Compose screen, compact 2x text, saved-state and listener lifecycle acceptance. */
class ClientJoinRestorationAcceptanceTest {
    @get:Rule val compose = createComposeRule()

    @Test
    fun approvalSyncBufferReconnectAndRestorationKeepCancelPinnedWithoutResubmittingJoin() {
        val session = SessionInfo(
            id = "client-restoration-session",
            name = "Warehouse Silent Disco",
            hostDeviceName = "Host phone",
            approvalMode = ApprovalMode.INVITE_CODE,
            inviteCodeRequired = true,
        )
        var uiState by mutableStateOf(
            AppUiState(
                selectedRole = AppRole.LISTENER,
                selectedSession = session,
                listenerState = ListenerLifecycleState.AWAITING_APPROVAL,
                connectionProgress = ConnectionProgressState(
                    currentState = ListenerLifecycleState.AWAITING_APPROVAL,
                    discovered = true,
                    requested = true,
                    inviteCode = "4826",
                ),
            ),
        )
        var joinCalls = 0
        var cancelCalls = 0
        var retryCalls = 0
        val restore = StateRestorationTester(compose)
        restore.setContent {
            CompositionLocalProvider(LocalDensity provides Density(1f, 2f)) {
                SilentDiscoTheme {
                    Box(Modifier.size(320.dp, 480.dp)) {
                        SessionJoinScreen(
                            uiState = uiState,
                            onInviteCodeChanged = {},
                            onJoin = { joinCalls++ },
                            onCancel = { cancelCalls++ },
                            onRetry = { retryCalls++ },
                            onReturnToSessions = {},
                            onOpenSettings = {},
                            onShareSupportReport = {},
                        )
                    }
                }
            }
        }

        fun assertWaitingScreen() {
            compose.onNodeWithTag("session-join-screen").assertExists()
            compose.onNodeWithTag("session-join-progress").assertExists()
            compose.onNodeWithTag("session-request-join").assertDoesNotExist()
            compose.onNodeWithTag("session-join-cancel").assertIsDisplayed()
        }

        assertWaitingScreen()
        restore.emulateSavedInstanceStateRestore()
        assertWaitingScreen()
        compose.runOnIdle {
            assertThat(joinCalls).isEqualTo(0)
            assertThat(retryCalls).isEqualTo(0)
        }

        // The host has approved admission; clock synchronization must not
        // accidentally return the client to an editable invite-code form.
        compose.runOnIdle {
            uiState = uiState.copy(
                listenerState = ListenerLifecycleState.SYNCING_CLOCK,
                connectionProgress = uiState.connectionProgress.copy(
                    currentState = ListenerLifecycleState.SYNCING_CLOCK,
                    approved = true,
                    connected = true,
                ),
            )
        }
        assertWaitingScreen()

        // Buffering and a transport reconnect are still client-side states:
        // the user retains Cancel, never a host Play/Stop or duplicate Join.
        compose.runOnIdle {
            uiState = uiState.copy(
                listenerState = ListenerLifecycleState.BUFFERING,
                connectionProgress = uiState.connectionProgress.copy(
                    currentState = ListenerLifecycleState.BUFFERING,
                    synced = true,
                ),
            )
        }
        assertWaitingScreen()
        compose.runOnIdle {
            uiState = uiState.copy(
                listenerState = ListenerLifecycleState.RECONNECTING,
                connectionProgress = uiState.connectionProgress.copy(
                    currentState = ListenerLifecycleState.RECONNECTING,
                ),
            )
        }
        assertWaitingScreen()
        restore.emulateSavedInstanceStateRestore()
        assertWaitingScreen()
        compose.onNodeWithTag("session-join-cancel").performClick()
        compose.runOnIdle {
            assertThat(cancelCalls).isEqualTo(1)
            assertThat(joinCalls).isEqualTo(0)
            assertThat(retryCalls).isEqualTo(0)
        }

        // A subsequent deliberate retry starts from the selected session.
        // A restored in-flight request must never dispatch Join by itself.
        compose.runOnIdle {
            uiState = uiState.copy(
                listenerState = ListenerLifecycleState.SESSION_SELECTED,
                connectionProgress = ConnectionProgressState(
                    currentState = ListenerLifecycleState.SESSION_SELECTED,
                    discovered = true,
                    inviteCode = "4826",
                ),
            )
        }
        compose.onNodeWithTag("session-request-join").assertIsDisplayed()
        restore.emulateSavedInstanceStateRestore()
        compose.runOnIdle { assertThat(joinCalls).isEqualTo(0) }
        compose.onNodeWithTag("session-request-join").performClick()
        compose.runOnIdle { assertThat(joinCalls).isEqualTo(1) }
    }
}
