package com.ekkus.silentdisco

import android.net.Uri
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.HostFormState
import com.ekkus.silentdisco.app.StorageInitializationState
import com.ekkus.silentdisco.core.model.AppRole
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.model.SyncState
import com.ekkus.silentdisco.feature.diagnostics.ConnectionHelpScreen
import com.ekkus.silentdisco.feature.host.HostAccessSetupScreen
import com.ekkus.silentdisco.feature.host.HostMusicSetupScreen
import com.ekkus.silentdisco.feature.listener.ListenerPlaybackV2Screen
import com.ekkus.silentdisco.feature.listener.NearbySessionsScreen
import com.ekkus.silentdisco.feature.settings.SettingsScreen
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import org.junit.Rule
import org.junit.Test

class AdaptiveLayoutCoverageTest {
    @get:Rule
    val composeRule = createComposeRule()

    private val selectedAudio = SelectedAudioFile(
        uri = Uri.parse("content://test/music/set.wav"),
        displayName = "Set.wav",
        mimeType = "audio/wav",
        sizeBytes = 4_000_000L,
    )

    private val hostForm = HostFormState(
        sessionName = "Rooftop Disco",
        approvalMode = ApprovalMode.MANUAL,
        selectedAudio = selectedAudio,
    )

    private val session = SessionInfo(
        id = "session-adaptive",
        name = "Rooftop Disco",
        hostDeviceName = "Host phone",
        approvalMode = ApprovalMode.MANUAL,
        inviteCodeRequired = false,
    )

    @Test
    fun hostMusicPrimaryActionRemainsReachableAtTwoHundredPercentFontScale() {
        composeRule.setContent {
            AdaptiveSurface(width = 360.dp, height = 640.dp, fontScale = 2f) {
                HostMusicSetupScreen(
                    uiState = readyState().copy(hostForm = hostForm),
                    onBack = {},
                    onSessionNameChanged = {},
                    onChooseAudio = {},
                    onNext = {},
                )
            }
        }

        composeRule.onNodeWithTag("host-music-next").performScrollTo().assertIsDisplayed()
    }

    @Test
    fun hostAccessPrimaryActionRemainsReachableAtTwoHundredPercentFontScale() {
        composeRule.setContent {
            AdaptiveSurface(width = 360.dp, height = 640.dp, fontScale = 2f) {
                HostAccessSetupScreen(
                    uiState = readyState().copy(hostForm = hostForm),
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

        composeRule.onNodeWithTag("host-start-session").performScrollTo().assertIsDisplayed()
    }

    @Test
    fun listenerPlaybackActionsRemainReachableInSmallLargeTextWindow() {
        composeRule.setContent {
            AdaptiveSurface(width = 320.dp, height = 480.dp, fontScale = 2f) {
                ListenerPlaybackV2Screen(
                    uiState = readyState().copy(
                        selectedRole = AppRole.LISTENER,
                        hostForm = hostForm,
                        selectedSession = session,
                        listenerState = ListenerLifecycleState.DESYNCED,
                        listenerPlaybackState = PlaybackState.PLAYING,
                        listenerSyncState = SyncState(confidence = SyncQualityBadge.POOR),
                        localVolume = 0.65f,
                    ),
                    onBackRequest = {},
                    onVolumeChanged = {},
                    onFixConnection = {},
                    onLeaveRequest = {},
                )
            }
        }

        composeRule.onNodeWithTag("listener-fix-connection").performScrollTo().assertIsDisplayed()
        composeRule.onNodeWithTag("listener-leave").performScrollTo().assertIsDisplayed()
    }

    @Test
    fun nearbySessionResultsRemainReachableInLandscapeWindow() {
        composeRule.setContent {
            AdaptiveSurface(width = 640.dp, height = 360.dp) {
                NearbySessionsScreen(
                    uiState = readyState().copy(discoveredSessions = listOf(session)),
                    permissionRequired = false,
                    onBack = {},
                    onRequestPermission = {},
                    onRefresh = {},
                    onSelectSession = {},
                )
            }
        }

        composeRule.onNodeWithTag("nearby-results").assertIsDisplayed()
        composeRule.onNodeWithText("Join").performScrollTo().assertIsDisplayed()
    }

    @Test
    fun connectionHelpActionsRemainReachableInLandscapeWindow() {
        composeRule.setContent {
            AdaptiveSurface(width = 640.dp, height = 360.dp) {
                ConnectionHelpScreen(
                    uiState = readyState().copy(
                        selectedRole = AppRole.LISTENER,
                        selectedSession = session,
                        listenerState = ListenerLifecycleState.DESYNCED,
                        listenerPlaybackState = PlaybackState.PLAYING,
                    ),
                    onBack = {},
                    onResynchronize = {},
                    onReconnect = {},
                    onShareSupportReport = {},
                    onAdvancedDiagnostics = {},
                )
            }
        }

        composeRule.onNodeWithTag("connection-help-resync").performScrollTo().assertIsDisplayed()
        composeRule.onNodeWithTag("connection-help-advanced").performScrollTo().assertIsDisplayed()
    }

    @Test
    fun settingsTroubleshootingRemainsReachableAtTabletWidth() {
        composeRule.setContent {
            AdaptiveSurface(width = 840.dp, height = 640.dp) {
                SettingsScreen(
                    uiState = readyState(),
                    trustedDeviceManagementAvailable = false,
                    onBack = {},
                    onOpenSystemSettings = {},
                    onOpenTrustedDevices = {},
                    onOpenAdvancedDiagnostics = {},
                )
            }
        }

        composeRule.onNodeWithTag("settings-troubleshooting").performScrollTo().assertIsDisplayed()
        composeRule.onNodeWithText("Advanced diagnostics").performScrollTo().assertIsDisplayed()
    }

    private fun readyState(): AppUiState = AppUiState(
        storageState = StorageInitializationState.READY,
    )
}

@Composable
private fun AdaptiveSurface(
    width: Dp,
    height: Dp,
    fontScale: Float = 1f,
    content: @Composable () -> Unit,
) {
    CompositionLocalProvider(
        LocalDensity provides Density(density = 1f, fontScale = fontScale),
    ) {
        SilentDiscoTheme {
            Box(modifier = Modifier.size(width = width, height = height)) {
                content()
            }
        }
    }
}
