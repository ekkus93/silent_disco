package com.ekkus.silentdisco

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.ConnectionProgressState
import com.ekkus.silentdisco.app.HostFormState
import com.ekkus.silentdisco.app.StorageInitializationState
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.ekkus.silentdisco.feature.home.RoleFirstHomeScreen
import com.ekkus.silentdisco.feature.host.HostMusicSetupScreen
import com.ekkus.silentdisco.feature.listener.SessionJoinScreen
import com.ekkus.silentdisco.feature.startup.StartupGateScreen
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import org.junit.Rule
import org.junit.Test

class RedesignedUiScreenTest {
    @get:Rule
    val composeRule = createComposeRule()

    @Test
    fun startupLoadingIsFocusedAndBlocking() {
        composeRule.setContent {
            SilentDiscoTheme {
                StartupGateScreen(
                    uiState = AppUiState(storageState = StorageInitializationState.INITIALIZING),
                    onRetry = {},
                    onShareSupportReport = {},
                )
            }
        }

        composeRule.onNodeWithTag("startup-loading").assertIsDisplayed()
        composeRule.onNodeWithText("Getting Silent Disco ready…").assertIsDisplayed()
    }

    @Test
    fun startupRecoverableFailureOffersRetry() {
        composeRule.setContent {
            SilentDiscoTheme {
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

        composeRule.onNodeWithTag("startup-recoverable").assertIsDisplayed()
        composeRule.onNodeWithTag("startup-retry").assertIsEnabled()
    }

    @Test
    fun startupFatalFailureHasNoContinueAction() {
        composeRule.setContent {
            SilentDiscoTheme {
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
        composeRule.onNodeWithText("Share support report").assertIsDisplayed()
        composeRule.onNodeWithText("Continue").assertDoesNotExist()
    }

    @Test
    fun readyHomePresentsRoleFirstActionsWithoutStorageCard() {
        composeRule.setContent {
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

        composeRule.onNodeWithTag("home-host").assertIsDisplayed()
        composeRule.onNodeWithTag("home-join").assertIsDisplayed()
        composeRule.onNodeWithText("Persistent storage").assertDoesNotExist()
        composeRule.onNodeWithText("Grant / Refresh Permissions").assertDoesNotExist()
    }

    @Test
    fun hostMusicNextRequiresAudioAndSessionName() {
        composeRule.setContent {
            SilentDiscoTheme {
                HostMusicSetupScreen(
                    uiState = AppUiState(
                        storageState = StorageInitializationState.READY,
                        hostForm = HostFormState(sessionName = ""),
                    ),
                    onBack = {},
                    onSessionNameChanged = {},
                    onChooseAudio = {},
                    onNext = {},
                )
            }
        }

        composeRule.onNodeWithTag("host-music-next").assertIsNotEnabled()
    }

    @Test
    fun hostMusicNextEnablesWhenRequiredInputsExist() {
        composeRule.setContent {
            SilentDiscoTheme {
                HostMusicSetupScreen(
                    uiState = AppUiState(
                        storageState = StorageInitializationState.READY,
                        hostForm = HostFormState(
                            sessionName = "Phillip’s Silent Disco",
                            selectedAudio = SelectedAudioFile(
                                uri = "content://test/audio",
                                displayName = "music.flac",
                                mimeType = "audio/flac",
                            ),
                        ),
                    ),
                    onBack = {},
                    onSessionNameChanged = {},
                    onChooseAudio = {},
                    onNext = {},
                )
            }
        }

        composeRule.onNodeWithTag("host-music-next").assertIsEnabled()
    }

    @Test
    fun requestedJoinShowsWaitingForApproval() {
        composeRule.setContent {
            SilentDiscoTheme {
                SessionJoinScreen(
                    uiState = AppUiState(
                        storageState = StorageInitializationState.READY,
                        listenerState = ListenerLifecycleState.CONNECTING,
                        connectionProgress = ConnectionProgressState(
                            currentState = ListenerLifecycleState.CONNECTING,
                            discovered = true,
                            requested = true,
                            approved = false,
                        ),
                    ),
                    onInviteCodeChanged = {},
                    onJoin = {},
                    onCancel = {},
                    onRetry = {},
                    onReturnToSessions = {},
                )
            }
        }

        composeRule.onNodeWithTag("session-join-progress").assertIsDisplayed()
        composeRule.onNodeWithText("Waiting for host approval").assertIsDisplayed()
    }
}
