package com.ekkus.silentdisco

import android.net.Uri
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import com.ekkus.silentdisco.app.AppRoutes
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.HostFormState
import com.ekkus.silentdisco.app.navigateSingleTop
import com.ekkus.silentdisco.app.returnFromHostReadyToAccess
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.ekkus.silentdisco.feature.host.HostAccessSetupScreen
import com.ekkus.silentdisco.feature.host.HostMusicSetupScreen
import com.ekkus.silentdisco.feature.host.HostReadyScreen
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import com.google.common.truth.Truth.assertThat
import org.junit.Rule
import org.junit.Test

/** Real Compose Navigation and saved-state acceptance, no simulated phone hardware. */
class HostSetupBackNavigationTest {
    @get:Rule val compose = createComposeRule()

    private fun fixture(): AppUiState = AppUiState(
        hostForm = HostFormState(
            sessionName = "Warehouse Silent Disco",
            selectedAudio = SelectedAudioFile(
                uri = Uri.parse("content://fixture/opening.wav"),
                displayName = "Opening song",
                mimeType = "audio/wav",
                sizeBytes = 1_024,
            ),
            approvalMode = ApprovalMode.INVITE_CODE,
            inviteCode = "4826",
        ),
    )

    @Test
    fun compactLargeTextMusicAccessReadyBackReopensAccessAndKeepsStartSingleShot() {
        val state = fixture()
        var starts = 0
        val restore = StateRestorationTester(compose)
        restore.setContent {
            CompositionLocalProvider(LocalDensity provides Density(1f, 1.5f)) {
                SilentDiscoTheme {
                    Box(Modifier.size(320.dp, 480.dp)) {
                        val nav = rememberNavController()
                        NavHost(
                            navController = nav,
                            startDestination = AppRoutes.HostMusicSetup,
                            modifier = Modifier.fillMaxSize(),
                        ) {
                            composable(AppRoutes.HostMusicSetup) {
                                HostMusicSetupScreen(
                                    uiState = state,
                                    onBack = {},
                                    onSessionNameChanged = {},
                                    onChooseAudio = {},
                                    onNext = { nav.navigateSingleTop(AppRoutes.HostAccessSetup) },
                                )
                            }
                            composable(AppRoutes.HostAccessSetup) {
                                HostAccessSetupScreen(
                                    uiState = state,
                                    onBack = { nav.popBackStack() },
                                    onApprovalModeChanged = {},
                                    onInviteCodeChanged = {},
                                    onGenerateCode = {},
                                    onNext = { nav.navigateSingleTop(AppRoutes.HostReady) },
                                )
                            }
                            composable(AppRoutes.HostReady) {
                                HostReadyScreen(
                                    uiState = state,
                                    onBack = { nav.returnFromHostReadyToAccess() },
                                    onStartSession = { starts++ },
                                    onOpenSettings = {},
                                    onShareSupportReport = {},
                                )
                            }
                        }
                    }
                }
            }
        }
        compose.onNodeWithTag("host-music-next").assertIsEnabled().performClick()
        compose.onNodeWithTag("host-access-next").assertIsEnabled().performClick()
        compose.onNodeWithTag("host-ready-screen").assertExists()

        restore.emulateSavedInstanceStateRestore()
        compose.onNodeWithTag("host-ready-screen").assertExists()
        compose.onNodeWithContentDescription("Back").performClick()

        // This used to restore the Access entry's saved in-flight lock:
        // a user editing Ready could never return to Ready again.
        compose.onNodeWithTag("host-access-next").assertIsEnabled().performClick()
        compose.onNodeWithTag("host-ready-screen").assertExists()
        compose.onNodeWithTag("host-start-session").assertIsEnabled().performClick()
        compose.onNodeWithTag("host-start-session").assertIsNotEnabled()
        restore.emulateSavedInstanceStateRestore()
        compose.onNodeWithTag("host-start-session").assertIsNotEnabled()
        compose.runOnIdle { assertThat(starts).isEqualTo(1) }
    }

    @Test
    fun invalidFailedReadyDoesNotOfferAnInertRetryButStillExposesPermissionRepair() {
        var starts = 0
        var settings = 0
        var state by androidx.compose.runtime.mutableStateOf(
            AppUiState(
                hostState = HostLifecycleState.ERROR,
                lastError = "Unable to create session",
            ),
        )
        compose.setContent {
            SilentDiscoTheme {
                HostReadyScreen(
                    uiState = state,
                    onBack = {},
                    onStartSession = { starts++ },
                    onOpenSettings = { settings++ },
                    onShareSupportReport = {},
                )
            }
        }
        compose.onNodeWithTag("host-start-session").assertIsNotEnabled()
        compose.runOnIdle {
            state = state.copy(lastError = "Nearby permission required")
        }
        compose.onNodeWithTag("host-start-session").assertIsEnabled().performClick()
        compose.runOnIdle {
            assertThat(starts).isEqualTo(0)
            assertThat(settings).isEqualTo(1)
        }
    }
}
