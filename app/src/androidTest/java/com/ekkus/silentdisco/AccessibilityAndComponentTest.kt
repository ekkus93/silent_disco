package com.ekkus.silentdisco

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Composable
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.unit.Density
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.HostQueueItem
import com.ekkus.silentdisco.app.HostFormState
import com.ekkus.silentdisco.app.StorageInitializationState
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.feature.home.RoleFirstHomeScreen
import com.ekkus.silentdisco.feature.host.HostAccessSetupScreen
import com.ekkus.silentdisco.feature.host.HostDashboardScreen
import com.ekkus.silentdisco.feature.host.HostMusicSetupScreen
import com.ekkus.silentdisco.feature.host.HostQueueManagementScreen
import com.ekkus.silentdisco.feature.host.HostReadyScreen
import com.ekkus.silentdisco.feature.listener.NearbySessionsScreen
import com.ekkus.silentdisco.feature.listener.SessionJoinScreen
import com.ekkus.silentdisco.ui.components.ConfirmationSheet
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import org.junit.Rule
import org.junit.Test
import android.net.Uri

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

    @Test
    fun hostMusicChooseAudioActionMeetsFortyEightDpMinimum() = assertTouchTarget("host-choose-audio") {
        HostMusicSetupScreen(AppUiState(), {}, {}, {}, onNext = {})
    }

    @Test
    fun hostAccessNextActionMeetsFortyEightDpMinimum() = assertTouchTarget("host-access-next") {
        HostAccessSetupScreen(AppUiState(), {}, {}, {}, {}, onNext = {})
    }

    @Test
    fun hostReadyStartActionMeetsFortyEightDpMinimum() = assertTouchTarget("host-start-session") {
        HostReadyScreen(validHostState(), {}, {}, {}, {})
    }

    @Test
    fun hostDashboardPrimaryActionsMeetFortyEightDpTouchTargetMinimum() {
        var densityScale = 1f
        composeRule.setContent {
            densityScale = LocalDensity.current.density
            SilentDiscoTheme {
                HostDashboardScreen(
                    uiState = AppUiState(
                        hostPlaybackState = PlaybackState.PLAYING,
                        hostQueue = listOf(HostQueueItem("current", "source", "Current song", null, 60_000L)),
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
                    onAddSongs = {},
                    onManageQueue = {},
                )
            }
        }

        listOf(
            composeRule.onNodeWithContentDescription("Add songs"),
            composeRule.onNodeWithContentDescription("Manage queue"),
            composeRule.onNodeWithText("Invite"),
            composeRule.onNodeWithText("Pause"),
            composeRule.onNodeWithText("Stop"),
        ).forEachIndexed { index, node ->
            val label = listOf("Add songs", "Manage queue", "Invite", "Pause", "Stop")[index]
            val bounds = node.fetchSemanticsNode().boundsInRoot
            assertWithMessage("$label width").that(bounds.width / densityScale).isAtLeast(48f)
            assertWithMessage("$label height").that(bounds.height / densityScale).isAtLeast(48f)
        }
    }

    @Test
    fun clientJoinActionMeetsFortyEightDpMinimum() = assertTouchTarget("session-request-join") {
        SessionJoinScreen(
            AppUiState(
                selectedSession = SessionInfo(
                    id = "session-1",
                    name = "Weekend Mix",
                    hostDeviceName = "Alex's phone",
                    approvalMode = ApprovalMode.MANUAL,
                    inviteCodeRequired = false,
                ),
                listenerState = ListenerLifecycleState.SESSION_SELECTED,
            ),
            {}, {}, {}, {}, {}, {}, {},
        )
    }

    @Test
    fun nearbyScanActionMeetsFortyEightDpMinimum() = assertTouchTarget("nearby-scan-qr") {
        NearbySessionsScreen(AppUiState(), false, {}, {}, {}, {}, onScanQr = {})
    }

    private fun validHostState(): AppUiState {
        val selectedSong = SelectedAudioFile(
            uri = Uri.parse("content://test/silent-disco.wav"),
            displayName = "silent-disco.wav",
            mimeType = "audio/wav",
            sizeBytes = 1_024,
        )
        return AppUiState(
            hostForm = HostFormState(sessionName = "Weekend Mix", selectedAudio = selectedSong),
            hostState = HostLifecycleState.IDLE,
        )
    }

    private fun assertTouchTarget(tag: String, content: @Composable () -> Unit) {
        var densityScale = 1f
        composeRule.setContent {
            densityScale = LocalDensity.current.density
            SilentDiscoTheme(content = content)
        }
        val bounds = composeRule.onNodeWithTag(tag).fetchSemanticsNode().boundsInRoot
        assertWithMessage("$tag width").that(bounds.width / densityScale).isAtLeast(48f)
        assertWithMessage("$tag height").that(bounds.height / densityScale).isAtLeast(48f)
    }
}
