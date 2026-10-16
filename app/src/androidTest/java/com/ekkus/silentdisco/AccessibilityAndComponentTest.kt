package com.ekkus.silentdisco

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.unit.Density
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.HostQueueItem
import com.ekkus.silentdisco.app.StorageInitializationState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.feature.home.RoleFirstHomeScreen
import com.ekkus.silentdisco.feature.host.HostQueueManagementScreen
import com.ekkus.silentdisco.ui.components.ConfirmationSheet
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import org.junit.Rule
import org.junit.Test

class AccessibilityAndComponentTest {
    @get:Rule
    val composeRule = createComposeRule()

    @Test
    fun destructiveConfirmationKeepsSafeActionEnabledAndDisablesDuplicateSubmit() {
        var confirmations = 0
        composeRule.setContent {
            SilentDiscoTheme {
                ConfirmationSheet(
                    visible = true,
                    title = "End this session?",
                    detail = "Playback will stop.",
                    safeActionLabel = "Keep hosting",
                    destructiveActionLabel = "End session",
                    onDismiss = {},
                    onConfirm = { confirmations += 1 },
                    testTag = "test-confirmation",
                )
            }
        }

        // Managed devices run in touch mode, where keyboard focus is not a stable initial-window
        // contract. Verify the safe action is available and the destructive action is single-shot.
        composeRule.onNodeWithTag("test-confirmation-safe").assertIsEnabled()
        composeRule.onNodeWithTag("test-confirmation-destructive").performClick()
        composeRule.onNodeWithTag("test-confirmation-destructive").assertIsNotEnabled()
        composeRule.runOnIdle { assertThat(confirmations).isEqualTo(1) }
    }

    @Test
    fun roleFirstHomeRemainsScrollableAtTwoHundredPercentFontScale() {
        composeRule.setContent {
            CompositionLocalProvider(LocalDensity provides Density(density = 1f, fontScale = 2f)) {
                SilentDiscoTheme {
                    RoleFirstHomeScreen(
                        uiState = AppUiState(storageState = StorageInitializationState.READY),
                        onHostClick = {},
                        onJoinClick = {},
                        onSettingsClick = {},
                        onRetryStorage = {},
                    )
                }
            }
        }

        composeRule.onNodeWithTag("home-host").assertIsDisplayed()
        composeRule.onNodeWithTag("home-join").performScrollTo().assertIsDisplayed()
    }

    @Test
    fun hostQueuePrimaryAndEditActionsMeetFortyEightDpTouchTargetMinimum() {
        var densityScale = 1f
        composeRule.setContent {
            densityScale = LocalDensity.current.density
            SilentDiscoTheme {
                HostQueueManagementScreen(
                    uiState = AppUiState(
                        hostPlaybackState = PlaybackState.PLAYING,
                        hostQueue = listOf(
                            HostQueueItem("current", "source-1", "Current song", null, null),
                            HostQueueItem("upcoming", "source-2", "Upcoming song", null, null),
                        ),
                    ),
                    onBack = {},
                    onAddSongs = {},
                    onReorderQueue = {},
                    onRemoveQueueEntry = {},
                )
            }
        }

        listOf(
            "host-queue-add-songs",
            "host-queue-move-down-upcoming",
            "host-queue-remove-upcoming",
        ).forEach { tag ->
            val bounds = composeRule.onNodeWithTag(tag).fetchSemanticsNode().boundsInRoot
            assertWithMessage("$tag width").that(bounds.width / densityScale).isAtLeast(48f)
            assertWithMessage("$tag height").that(bounds.height / densityScale).isAtLeast(48f)
        }
    }
}
