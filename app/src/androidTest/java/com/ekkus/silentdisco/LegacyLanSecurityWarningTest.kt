package com.ekkus.silentdisco

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.assertExists
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
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
            .performScrollTo().assertIsNotEnabled()
    }
}
