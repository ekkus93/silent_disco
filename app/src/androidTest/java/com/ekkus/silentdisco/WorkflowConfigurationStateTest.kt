package com.ekkus.silentdisco

import android.net.Uri
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.HostFormState
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.ekkus.silentdisco.feature.host.HostAccessSetupScreen
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import com.google.common.truth.Truth.assertThat
import org.junit.Rule
import org.junit.Test

class WorkflowConfigurationStateTest {
    @get:Rule
    val composeRule = createComposeRule()

    @Test
    fun hostAccessInFlightSubmitRemainsDisabledAfterStateRestoration() {
        var nextCalls = 0
        val stateRestorationTester = StateRestorationTester(composeRule)
        stateRestorationTester.setContent {
            SilentDiscoTheme {
                HostAccessSetupScreen(
                    uiState = AppUiState(
                        hostForm = HostFormState(
                            selectedAudio = SelectedAudioFile(
                                uri = Uri.parse("content://test/host-access.wav"),
                                displayName = "host-access.wav",
                                mimeType = "audio/wav",
                                sizeBytes = 1_024,
                            ),
                        ),
                    ),
                    onBack = {},
                    onApprovalModeChanged = {},
                    onInviteCodeChanged = {},
                    onGenerateCode = {},
                    onNext = { nextCalls += 1 },
                )
            }
        }

        composeRule.onNodeWithTag("host-access-next").performClick()
        composeRule.onNodeWithTag("host-access-next").assertIsNotEnabled()
        composeRule.runOnIdle { assertThat(nextCalls).isEqualTo(1) }

        stateRestorationTester.emulateSavedInstanceStateRestore()

        composeRule.onNodeWithTag("host-access-next").assertIsNotEnabled()
        composeRule.onNodeWithTag("host-access-next").performClick()
        composeRule.runOnIdle { assertThat(nextCalls).isEqualTo(1) }
    }
}
