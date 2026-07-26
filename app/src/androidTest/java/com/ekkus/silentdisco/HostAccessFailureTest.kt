package com.ekkus.silentdisco

import android.net.Uri
import androidx.compose.ui.test.assertDoesNotExist
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.HostFormState
import com.ekkus.silentdisco.app.StorageInitializationState
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.ekkus.silentdisco.feature.host.HostAccessSetupScreen
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import com.google.common.truth.Truth.assertThat
import org.junit.Rule
import org.junit.Test

class HostAccessFailureTest {
    @get:Rule
    val composeRule = createComposeRule()

    private fun validForm() = HostFormState(
        sessionName = "Kitchen Disco",
        selectedAudio = SelectedAudioFile(
            uri = Uri.parse("content://test/audio"),
            displayName = "music.flac",
            mimeType = "audio/flac",
            sizeBytes = 1024L,
        ),
        approvalMode = ApprovalMode.MANUAL,
    )

    @Test
    fun permissionFailureShowsSettingsAndRetryWithoutDuplicateStartButton() {
        composeRule.setContent {
            SilentDiscoTheme {
                HostAccessSetupScreen(
                    uiState = AppUiState(
                        storageState = StorageInitializationState.READY,
                        hostState = HostLifecycleState.ERROR,
                        hostForm = validForm(),
                        lastError = "Missing nearby connectivity permissions for advertising",
                    ),
                    onBack = {},
                    onApprovalModeChanged = {},
                    onInviteCodeChanged = {},
                    onGenerateCode = {},
                    onStartSession = {},
                    onOpenSettings = {},
                    onShareSupportReport = {},
                )
            }
        }

        composeRule.onNodeWithTag("host-start-problem").assertIsDisplayed()
        composeRule.onNodeWithText("Open Settings").assertIsDisplayed()
        composeRule.onNodeWithText("Retry").assertIsDisplayed()
        composeRule.onNodeWithTag("host-start-session").assertDoesNotExist()
    }

    @Test
    fun transportFailureShowsRetryAndSupportReport() {
        composeRule.setContent {
            SilentDiscoTheme {
                HostAccessSetupScreen(
                    uiState = AppUiState(
                        storageState = StorageInitializationState.READY,
                        hostState = HostLifecycleState.ERROR,
                        hostForm = validForm(),
                        lastError = "Wi-Fi Direct host could not start",
                    ),
                    onBack = {},
                    onApprovalModeChanged = {},
                    onInviteCodeChanged = {},
                    onGenerateCode = {},
                    onStartSession = {},
                    onOpenSettings = {},
                    onShareSupportReport = {},
                )
            }
        }

        composeRule.onNodeWithText("The session could not be started").assertIsDisplayed()
        composeRule.onNodeWithText("Retry").assertIsDisplayed()
        composeRule.onNodeWithText("Share support report").assertIsDisplayed()
    }

    @Test
    fun startSessionSubmissionCannotBeIssuedTwice() {
        var starts = 0
        composeRule.setContent {
            SilentDiscoTheme {
                HostAccessSetupScreen(
                    uiState = AppUiState(
                        storageState = StorageInitializationState.READY,
                        hostState = HostLifecycleState.IDLE,
                        hostForm = validForm(),
                    ),
                    onBack = {},
                    onApprovalModeChanged = {},
                    onInviteCodeChanged = {},
                    onGenerateCode = {},
                    onStartSession = { starts += 1 },
                    onOpenSettings = {},
                    onShareSupportReport = {},
                )
            }
        }

        composeRule.onNodeWithTag("host-start-session").performClick()
        composeRule.onNodeWithTag("host-start-session").assertIsNotEnabled()
        composeRule.runOnIdle { assertThat(starts).isEqualTo(1) }
    }
}
