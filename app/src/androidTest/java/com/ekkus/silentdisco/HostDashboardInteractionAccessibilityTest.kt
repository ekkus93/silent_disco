package com.ekkus.silentdisco

import android.os.SystemClock
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.assertContentDescriptionEquals
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.JoinApprovalAction
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.JoinApprovalState
import com.ekkus.silentdisco.core.model.JoinRequest
import com.ekkus.silentdisco.core.model.ListenerInfo
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.model.TrustState
import com.ekkus.silentdisco.feature.host.HostDashboardScreen
import com.ekkus.silentdisco.feature.host.HostDashboardTab
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import com.google.common.truth.Truth.assertThat
import org.junit.Rule
import org.junit.Test

/**
 * The live host must keep individual listener controls identifiable and
 * reachable at compact, large-text sizes; approval callbacks remain
 * single-flight across Compose saved-state restoration and actor timeouts.
 */
class HostDashboardInteractionAccessibilityTest {
    @get:Rule val compose = createComposeRule()

    @Test
    fun compactLargeTextListenerActionsAreDistinctAndMeetTouchMinimum() {
        val listeners = listOf("listener-one" to "First phone", "listener-two" to "Second phone")
        val removed = mutableListOf<String>()
        val state = AppUiState(
            hostState = HostLifecycleState.READY,
            approvedListeners = listeners.map { (id, name) ->
                ListenerInfo(
                    deviceId = id,
                    displayName = name,
                    joinState = JoinApprovalState.APPROVED,
                    trustState = TrustState.SESSION_ONLY,
                    connectionState = TransportConnectionState.CONNECTED,
                    listenerState = ListenerLifecycleState.PLAYING,
                    syncQuality = SyncQualityBadge.GOOD,
                )
            },
        )
        compose.setContent {
            CompositionLocalProvider(LocalDensity provides Density(1f, 1.5f)) {
                SilentDiscoTheme {
                    Box(Modifier.size(320.dp, 480.dp)) {
                        HostDashboardScreen(
                            uiState = state,
                            initialTab = HostDashboardTab.CONNECTED,
                            onBackRequest = {},
                            onInvite = {},
                            onApproval = { _, _ -> },
                            onRemoveListener = { removed += it },
                            onPlayPause = {},
                            onStop = {},
                            onEndSessionRequest = {},
                            onOpenConnectionHelp = {},
                            onAddDemoJoinRequest = {},
                        )
                    }
                }
            }
        }
        listeners.forEach { (id, name) ->
            val tag = "host-listener-actions-$id"
            compose.onNodeWithTag("host-dashboard-list")
                .performScrollToNode(hasTestTag(tag))
            val node = compose.onNodeWithTag(tag)
                .assertContentDescriptionEquals("Actions for $name")
            val bounds = node.fetchSemanticsNode().boundsInRoot
            assertThat(bounds.width).isAtLeast(48f)
            assertThat(bounds.height).isAtLeast(48f)
            node.performClick()
            compose.onNodeWithText("Disconnect").performClick()
        }
        compose.runOnIdle {
            assertThat(removed).containsExactly("listener-one", "listener-two").inOrder()
        }
    }

    @Test
    fun twoPendingApprovalsHaveDistinctSpokenTargetsAndDispatchOnlySelectedRequest() {
        val now = SystemClock.elapsedRealtime() + 3_600_000L
        val requests = listOf(
            JoinRequest("first-request", "session", "first-device", "First phone", null, now),
            JoinRequest("second-request", "session", "second-device", "Second phone", null, now),
        )
        val decisions = mutableListOf<Pair<String, JoinApprovalAction>>()
        compose.setContent {
            CompositionLocalProvider(LocalDensity provides Density(1f, 1.5f)) {
                SilentDiscoTheme {
                    Box(Modifier.size(320.dp, 480.dp)) {
                        HostDashboardScreen(
                            uiState = AppUiState(
                                hostState = HostLifecycleState.READY,
                                pendingJoinRequests = requests,
                            ),
                            onBackRequest = {},
                            onInvite = {},
                            onApproval = { request, action ->
                                decisions += request.requestId to action
                            },
                            onRemoveListener = {},
                            onPlayPause = {},
                            onStop = {},
                            onEndSessionRequest = {},
                            onOpenConnectionHelp = {},
                            onAddDemoJoinRequest = {},
                        )
                    }
                }
            }
        }

        val list = compose.onNodeWithTag("host-dashboard-list")
        list.performScrollToNode(hasContentDescription("Approve Second phone once"))
        compose.onNodeWithContentDescription("Approve Second phone once")
            .assertIsEnabled()
            .performClick()
        compose.runOnIdle {
            assertThat(decisions).containsExactly(
                "second-request" to JoinApprovalAction.APPROVE_ONCE,
            )
        }
        // An in-flight approval locks both cards, not just the one clicked;
        // the other listener cannot be inadvertently approved or rejected.
        list.performScrollToNode(hasContentDescription("Reject First phone"))
        compose.onNodeWithContentDescription("Reject First phone").assertIsNotEnabled()
        compose.onNodeWithContentDescription("Approve First phone once").assertIsNotEnabled()
        compose.runOnIdle { assertThat(decisions).hasSize(1) }
    }

    @Test
    fun approvalTimeoutAfterRestoreAllowsRetryWithoutDuplicateDispatch() {
        val first = JoinRequest(
            requestId = "request-one",
            sessionId = "test-session",
            listenerId = "phone-one",
            listenerName = "First phone",
            inviteCode = null,
            requestedAtMs = SystemClock.elapsedRealtime() + 3_600_000L,
        )
        val second = first.copy(
            requestId = "request-two",
            listenerId = "phone-two",
            listenerName = "Second phone",
        )
        var state by mutableStateOf(
            AppUiState(
                hostState = HostLifecycleState.READY,
                pendingJoinRequests = listOf(first),
            ),
        )
        val decisions = mutableListOf<Pair<String, JoinApprovalAction>>()
        val restoration = StateRestorationTester(compose)
        restoration.setContent {
            SilentDiscoTheme {
                HostDashboardScreen(
                    uiState = state,
                    initialTab = HostDashboardTab.REQUESTS,
                    onBackRequest = {},
                    onInvite = {},
                    onApproval = { request, action ->
                        decisions += request.requestId to action
                    },
                    onRemoveListener = {},
                    onPlayPause = {},
                    onStop = {},
                    onEndSessionRequest = {},
                    onOpenConnectionHelp = {},
                    onAddDemoJoinRequest = {},
                )
            }
        }

        compose.onNodeWithTag("host-dashboard-list")
            .performScrollToNode(hasText("Approve once"))
        compose.onNodeWithText("Approve once").assertIsEnabled().performClick()
        compose.onNodeWithTag("host-dashboard-list")
            .performScrollToNode(hasTestTag("approval-progress-request-one"))
        compose.onNodeWithText("Approve once").assertIsNotEnabled()
        restoration.emulateSavedInstanceStateRestore()
        compose.onNodeWithTag("host-dashboard-list")
            .performScrollToNode(hasText("Approve once"))
        compose.onNodeWithText("Approve once").assertIsNotEnabled()
        compose.runOnIdle {
            assertThat(decisions).containsExactly("request-one" to JoinApprovalAction.APPROVE_ONCE)
        }

        // A failed actor/network confirmation leaves the request pending,
        // but must release the saved in-flight latch for an explicit retry.
        compose.runOnIdle { state = state.copy(lastError = "Approval timed out") }
        compose.onNodeWithTag("host-dashboard-list")
            .performScrollToNode(hasText("Approve once"))
        compose.onNodeWithText("Approve once").assertIsEnabled().performClick()
        compose.runOnIdle {
            assertThat(decisions).containsExactly(
                "request-one" to JoinApprovalAction.APPROVE_ONCE,
                "request-one" to JoinApprovalAction.APPROVE_ONCE,
            ).inOrder()
        }

        // Authoritative completion removes the request. A new listener's
        // rejection must not be blocked by the prior in-flight decision.
        compose.runOnIdle {
            state = state.copy(
                lastError = null,
                pendingJoinRequests = listOf(second),
            )
        }
        compose.onNodeWithTag("approval-progress-request-one").assertDoesNotExist()
        compose.onNodeWithTag("host-dashboard-list")
            .performScrollToNode(hasText("Reject"))
        compose.onNodeWithText("Reject").assertIsEnabled().performClick()
        compose.runOnIdle {
            assertThat(decisions.last()).isEqualTo("request-two" to JoinApprovalAction.REJECT)
            assertThat(decisions).hasSize(3)
        }
    }

    @Test
    fun unacknowledgedApprovalTimesOutAfterRestoreAndUnlocksOtherRequest() {
        val requestedAt = SystemClock.elapsedRealtime() + 3_600_000L
        val first = JoinRequest(
            "timeout-first", "session", "phone-one", "First phone", null, requestedAt,
        )
        val second = JoinRequest(
            "timeout-second", "session", "phone-two", "Second phone", null, requestedAt,
        )
        val decisions = mutableListOf<Pair<String, JoinApprovalAction>>()
        val restoration = StateRestorationTester(compose)
        restoration.setContent {
            CompositionLocalProvider(LocalDensity provides Density(1f, 1.5f)) {
                SilentDiscoTheme {
                    Box(Modifier.size(320.dp, 480.dp)) {
                        HostDashboardScreen(
                            uiState = AppUiState(
                                hostState = HostLifecycleState.READY,
                                pendingJoinRequests = listOf(first, second),
                            ),
                            initialTab = HostDashboardTab.REQUESTS,
                            approvalConfirmationTimeoutMs = 3_000L,
                            onBackRequest = {},
                            onInvite = {},
                            onApproval = { request, action ->
                                decisions += request.requestId to action
                            },
                            onRemoveListener = {},
                            onPlayPause = {},
                            onStop = {},
                            onEndSessionRequest = {},
                            onOpenConnectionHelp = {},
                            onAddDemoJoinRequest = {},
                        )
                    }
                }
            }
        }

        val list = compose.onNodeWithTag("host-dashboard-list")
        list.performScrollToNode(hasContentDescription("Approve First phone once"))
        compose.onNodeWithContentDescription("Approve First phone once")
            .assertIsEnabled().performClick()
        restoration.emulateSavedInstanceStateRestore()
        list.performScrollToNode(hasContentDescription("Approve First phone once"))
        compose.onNodeWithContentDescription("Approve First phone once").assertIsNotEnabled()
        list.performScrollToNode(hasContentDescription("Reject Second phone"))
        compose.onNodeWithContentDescription("Reject Second phone").assertIsNotEnabled()
        compose.runOnIdle {
            assertThat(decisions).containsExactly(
                "timeout-first" to JoinApprovalAction.APPROVE_ONCE,
            )
        }

        // No actor error or request-list change arrives. The saved monotonic
        // deadline must release the latch without redispatching the first
        // decision, even though the composable was restored in-flight.
        list.performScrollToNode(hasContentDescription("Approve First phone once"))
        compose.waitUntil(8_000L) {
            compose.onAllNodesWithTag("approval-timeout-timeout-first")
                .fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("approval-timeout-timeout-first").assertExists()
        compose.onNodeWithContentDescription("Approve First phone once").assertIsEnabled()
        list.performScrollToNode(hasContentDescription("Reject Second phone"))
        compose.onNodeWithContentDescription("Reject Second phone")
            .assertIsEnabled().performClick()
        compose.runOnIdle {
            assertThat(decisions).containsExactly(
                "timeout-first" to JoinApprovalAction.APPROVE_ONCE,
                "timeout-second" to JoinApprovalAction.REJECT,
            ).inOrder()
        }
    }
}
