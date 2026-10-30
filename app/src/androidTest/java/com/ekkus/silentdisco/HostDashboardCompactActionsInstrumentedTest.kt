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
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.HostQueueItem
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.feature.host.HostDashboardScreen
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import com.google.common.truth.Truth.assertThat
import org.junit.Rule
import org.junit.Test

/** The production dashboard must not clip its session commands at large text. */
class HostDashboardCompactActionsInstrumentedTest {
    @get:Rule val compose = createComposeRule()

    @Test
    fun compactLargeTextActionsRemainReachableAndLandscapeRestoresDirectActions() {
        var compact by mutableStateOf(true)
        var addCalls = 0
        var manageCalls = 0
        var inviteCalls = 0
        val state = AppUiState(
            hostState = HostLifecycleState.READY,
            hostPlaybackState = PlaybackState.PLAYING,
            hostQueue = listOf(HostQueueItem("first", "source", "First track", null, null)),
        )
        compose.setContent {
            val width = if (compact) 320.dp else 640.dp
            val height = if (compact) 480.dp else 360.dp
            val fontScale = if (compact) 2f else 1.4f
            CompositionLocalProvider(LocalDensity provides Density(1f, fontScale)) {
                SilentDiscoTheme {
                    Box(Modifier.size(width, height)) {
                        HostDashboardScreen(
                            uiState = state,
                            onBackRequest = {},
                            onInvite = { inviteCalls++ },
                            onApproval = { _, _ -> },
                            onRemoveListener = {},
                            onPlayPause = {},
                            onStop = {},
                            onEndSessionRequest = {},
                            onOpenConnectionHelp = {},
                            onAddDemoJoinRequest = {},
                            onAddSongs = { addCalls++ },
                            onManageQueue = { manageCalls++ },
                        )
                    }
                }
            }
        }

        // No invisible icon-only actions compete with the title at 320 dp.
        compose.onNodeWithContentDescription("Add songs").assertDoesNotExist()
        compose.onNodeWithContentDescription("Manage queue").assertDoesNotExist()
        compose.onNodeWithTag("host-session-actions").assertIsDisplayed().performClick()
        compose.onNodeWithTag("host-menu-add-songs").assertIsDisplayed().performClick()
        compose.onNodeWithTag("host-session-actions").performClick()
        compose.onNodeWithTag("host-menu-manage-queue").assertIsDisplayed().performClick()
        compose.onNodeWithTag("host-session-actions").performClick()
        compose.onNodeWithTag("host-menu-invite").assertIsDisplayed().performClick()
        compose.runOnIdle {
            assertThat(addCalls).isEqualTo(1)
            assertThat(manageCalls).isEqualTo(1)
            assertThat(inviteCalls).isEqualTo(1)
            compact = false
        }

        // The same composition must expose direct commands when the viewport
        // becomes wide enough; callbacks and host state remain unchanged.
        compose.onNodeWithContentDescription("Add songs").assertIsDisplayed().performClick()
        compose.onNodeWithContentDescription("Manage queue").assertIsDisplayed().performClick()
        compose.onNodeWithTag("host-session-actions").assertIsDisplayed()
        compose.runOnIdle {
            assertThat(addCalls).isEqualTo(2)
            assertThat(manageCalls).isEqualTo(2)
            assertThat(inviteCalls).isEqualTo(1)
        }
    }
}
