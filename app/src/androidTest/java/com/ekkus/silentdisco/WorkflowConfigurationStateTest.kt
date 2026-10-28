package com.ekkus.silentdisco

import android.net.Uri
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsNotSelected
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.HostFormState
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.ekkus.silentdisco.feature.host.HostAccessSetupScreen
import com.ekkus.silentdisco.feature.host.HostReadyScreen
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import com.google.common.truth.Truth.assertThat
import org.junit.Rule
import org.junit.Test

class WorkflowConfigurationStateTest {
    @get:Rule
    val composeRule = createComposeRule()

    @Test
    fun compactLargeTextAccessChoicesExposeSelectedRadioStateAndKeepNextPinned() {
        var selections = 0
        var state by mutableStateOf(
            AppUiState(
                hostForm = HostFormState(
                    selectedAudio = SelectedAudioFile(
                        uri = Uri.parse("content://test/compact-access.wav"),
                        displayName = "compact-access.wav",
                        mimeType = "audio/wav",
                        sizeBytes = 1_024,
                    ),
                ),
            ),
        )
        composeRule.setContent {
            val density = LocalDensity.current
            CompositionLocalProvider(
                LocalDensity provides Density(density.density, 1.5f),
            ) {
                SilentDiscoTheme {
                    Box(modifier = androidx.compose.ui.Modifier.size(320.dp, 440.dp)) {
                        HostAccessSetupScreen(
                            uiState = state,
                            onBack = {},
                            onApprovalModeChanged = { mode ->
                                selections += 1
                                state = state.copy(
                                    hostForm = state.hostForm.copy(approvalMode = mode),
                                )
                            },
                            onInviteCodeChanged = {},
                            onGenerateCode = {},
                            onNext = {},
                        )
                    }
                }
            }
        }

        composeRule.onNodeWithTag("legacy-lan-security-warning").performScrollTo()
        composeRule.onNodeWithTag("legacy-lan-security-warning").assertExists()
        composeRule.onNodeWithTag("host-access-manual").assertIsSelected()
        composeRule.onNodeWithTag("host-access-invite_code")
            .performScrollTo()
            .assertIsNotSelected()
            .performClick()
        composeRule.onNodeWithTag("host-access-invite_code").assertIsSelected()
        composeRule.onNodeWithTag("host-access-manual").assertIsNotSelected()
        composeRule.onNodeWithTag("host-access-next").assertIsNotEnabled()
        composeRule.onNodeWithTag("host-access-trusted_devices_placeholder")
            .performScrollTo()
            .performClick()
        composeRule.onNodeWithTag("host-access-trusted_devices_placeholder").assertIsSelected()
        composeRule.onNodeWithTag("host-access-next").assertIsEnabled()
        composeRule.runOnIdle { assertThat(selections).isEqualTo(2) }
    }

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

    @Test
    fun hostAccessFailedSubmissionAfterRestoreShowsErrorAndAllowsSingleRetry() {
        var nextCalls = 0
        var state by mutableStateOf(
            AppUiState(
                hostForm = HostFormState(
                    selectedAudio = SelectedAudioFile(
                        uri = Uri.parse("content://test/retry.wav"),
                        displayName = "retry.wav",
                        mimeType = "audio/wav",
                        sizeBytes = 1_024,
                    ),
                ),
            ),
        )
        val restoration = StateRestorationTester(composeRule)
        restoration.setContent {
            SilentDiscoTheme {
                HostAccessSetupScreen(
                    uiState = state,
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
        restoration.emulateSavedInstanceStateRestore()
        composeRule.onNodeWithTag("host-access-next").assertIsNotEnabled()
        composeRule.runOnIdle { assertThat(nextCalls).isEqualTo(1) }

        // The ViewModel may reject an asynchronous session start without
        // leaving IDLE. The saved in-flight latch must then clear so the
        // visible, fixed-position action can be retried deliberately.
        composeRule.runOnIdle { state = state.copy(lastError = "Unable to start session") }
        composeRule.onNodeWithTag("host-access-error")
            .assertTextContains("Unable to start session")
        composeRule.onNodeWithTag("host-access-next").assertIsEnabled()
        composeRule.onNodeWithTag("host-access-next").performClick()
        composeRule.onNodeWithTag("host-access-next").assertIsNotEnabled()
        composeRule.runOnIdle { assertThat(nextCalls).isEqualTo(2) }
    }

    @Test
    fun hostReadyAsyncFailureAfterRestoreShowsRetryWithoutDuplicateStart() {
        var startCalls = 0
        var state by mutableStateOf(
            AppUiState(
                hostForm = HostFormState(
                    sessionName = "Warehouse Silent Disco",
                    selectedAudio = SelectedAudioFile(
                        uri = Uri.parse("content://test/ready-retry.wav"),
                        displayName = "ready-retry.wav",
                        mimeType = "audio/wav",
                        sizeBytes = 1_024,
                    ),
                ),
            ),
        )
        val restoration = StateRestorationTester(composeRule)
        restoration.setContent {
            SilentDiscoTheme {
                HostReadyScreen(
                    uiState = state,
                    onBack = {},
                    onStartSession = { startCalls += 1 },
                    onOpenSettings = {},
                    onShareSupportReport = {},
                )
            }
        }

        composeRule.onNodeWithTag("legacy-lan-security-warning").performScrollTo().assertExists()
        composeRule.onNodeWithTag("host-start-session").assertIsEnabled().performClick()
        composeRule.onNodeWithTag("host-start-session").assertIsNotEnabled()
        restoration.emulateSavedInstanceStateRestore()
        composeRule.onNodeWithTag("host-start-session").assertIsNotEnabled()
        composeRule.runOnIdle { assertThat(startCalls).isEqualTo(1) }

        // An asynchronous transport/start failure can leave the actor IDLE,
        // rather than ERROR. A saved in-flight latch must not strand the
        // primary action or hide the failure after the ViewModel reports it.
        composeRule.runOnIdle {
            state = state.copy(
                hostState = HostLifecycleState.IDLE,
                lastError = "Unable to start the session",
            )
        }
        composeRule.onNodeWithTag("host-start-session")
            .assertTextContains("Retry")
            .assertIsEnabled()
            .performClick()
        composeRule.onNodeWithTag("host-start-session").assertIsNotEnabled()
        composeRule.runOnIdle { assertThat(startCalls).isEqualTo(2) }
    }
}
