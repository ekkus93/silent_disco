package com.ekkus.silentdisco

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.core.model.ManualEndpointFormState
import com.ekkus.silentdisco.feature.listener.ManualEndpointScreen
import com.ekkus.silentdisco.feature.listener.SessionJoinScreen
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import org.junit.Rule
import org.junit.Test

/** Real Android Compose screens: security disclosures survive compact, large-text layouts. */
class LegacyLanSecurityWarningTest {
    @get:Rule val composeRule = createComposeRule()

    @Test
    fun listenerJoinDisclosesUnboundPlaintextTransportWithoutEnablingInvalidJoin() {
        composeRule.setContent {
            val density = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(density.density, 1.8f)) {
                SilentDiscoTheme {
                    Box(modifier = Modifier.size(340.dp, 400.dp)) {
                        SessionJoinScreen(
                            uiState = AppUiState(),
                            onInviteCodeChanged = {},
                            onJoin = {},
                            onCancel = {},
                            onRetry = {},
                            onReturnToSessions = {},
                            onOpenSettings = {},
                            onShareSupportReport = {},
                        )
                    }
                }
            }
        }
        composeRule.onNodeWithTag("legacy-lan-security-warning")
            .performScrollTo().assertExists()
        composeRule.onNodeWithText("Local network connection is not secured").assertExists()
        composeRule.onNodeWithTag("session-request-join").assertIsNotEnabled()
        composeRule.onNodeWithTag("session-join-cancel").assertExists()
    }

    @Test
    fun manualConnectionDisclosesPlaintextEvenWhenEndpointIsEmpty() {
        composeRule.setContent {
            val density = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(density.density, 1.8f)) {
                SilentDiscoTheme {
                    Box(modifier = Modifier.size(340.dp, 400.dp)) {
                        ManualEndpointScreen(
                            uiState = AppUiState(),
                            onBack = {},
                            onInputChanged = {},
                            onInviteCodeChanged = {},
                            onConnect = {},
                            onCancel = {},
                        )
                    }
                }
            }
        }
        composeRule.onNodeWithTag("legacy-lan-security-warning")
            .performScrollTo().assertExists()
        composeRule.onNodeWithText("Local network connection is not secured").assertExists()
        composeRule.onNodeWithTag("manual-endpoint-connect")
            .assertIsNotEnabled()
    }
    @Test
    fun compactLargeTextManualJoinKeepsFooterVisibleWhileInviteFieldScrolls() {
        composeRule.setContent {
            val density = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(density.density, 1.8f)) {
                SilentDiscoTheme {
                    Box(modifier = Modifier.size(340.dp, 400.dp)) {
                        ManualEndpointScreen(
                            uiState = AppUiState(
                                manualEndpointForm = ManualEndpointFormState(
                                    hostAddress = "192.168.1.50",
                                    sessionId = "session-1",
                                    inviteCodeRequired = true,
                                ),
                            ),
                            onBack = {},
                            onInputChanged = {},
                            onInviteCodeChanged = {},
                            onConnect = {},
                            onCancel = {},
                        )
                    }
                }
            }
        }
        // The input and invitation remain reachable by scrolling the form,
        // while the fixed primary/Cancel footer must not require scrolling.
        composeRule.onNodeWithTag("manual-endpoint-invite-code")
            .performScrollTo().assertIsDisplayed()
        composeRule.onNodeWithTag("manual-endpoint-connect")
            .assertIsDisplayed().assertIsNotEnabled()
        composeRule.onNodeWithTag("manual-endpoint-cancel").assertIsDisplayed()
        composeRule.onNodeWithTag("legacy-lan-security-warning")
            .performScrollTo().assertExists()
        composeRule.onNodeWithTag("manual-endpoint-cancel").assertIsDisplayed()
    }

}
