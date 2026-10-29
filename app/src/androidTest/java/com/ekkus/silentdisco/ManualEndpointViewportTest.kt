package com.ekkus.silentdisco

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.core.model.ManualConnectUiState
import com.ekkus.silentdisco.feature.listener.ManualEndpointScreen
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import com.google.common.truth.Truth.assertThat
import org.junit.Rule
import org.junit.Test

/** The manual-IP fallback must retain its Cancel/Connect actions in a tiny viewport. */
class ManualEndpointViewportTest {
    @get:Rule val composeRule = createComposeRule()

    @Test
    fun largeTextScrollKeepsPrimaryAndBackActionsVisibleWithoutValidPayload() {
        var backCalls = 0
        composeRule.setContent {
            val density = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(density.density, 1.8f)) {
                SilentDiscoTheme {
                    Box(modifier = Modifier.size(320.dp, 400.dp)) {
                        ManualEndpointScreen(
                            uiState = AppUiState(),
                            onBack = {},
                            onInputChanged = {},
                            onInviteCodeChanged = {},
                            onConnect = { error("invalid payload must not connect") },
                            onCancel = { backCalls++ },
                        )
                    }
                }
            }
        }
        composeRule.onNodeWithTag("manual-endpoint-connect")
            .assertIsDisplayed().assertIsNotEnabled()
        composeRule.onNodeWithTag("manual-endpoint-cancel").assertIsDisplayed()
        composeRule.onNodeWithTag("legacy-lan-security-warning")
            .performScrollTo().assertExists()
        composeRule.onNodeWithTag("manual-endpoint-cancel")
            .assertIsDisplayed().performClick()
        composeRule.runOnIdle { assertThat(backCalls).isEqualTo(1) }
    }

    @Test
    fun pendingManualConnectionKeepsCancelVisibleAfterSavedStateRestoration() {
        var cancelCalls = 0
        var state by mutableStateOf(
            AppUiState(manualConnectState = ManualConnectUiState.Connecting),
        )
        val restoration = StateRestorationTester(composeRule)
        restoration.setContent {
            val density = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(density.density, 1.6f)) {
                SilentDiscoTheme {
                    Box(modifier = Modifier.size(320.dp, 400.dp)) {
                        ManualEndpointScreen(
                            uiState = state,
                            onBack = {},
                            onInputChanged = {},
                            onInviteCodeChanged = {},
                            onConnect = {},
                            onCancel = { cancelCalls++ },
                        )
                    }
                }
            }
        }
        composeRule.onNodeWithTag("manual-endpoint-progress").assertExists()
        composeRule.onNodeWithTag("manual-endpoint-input").assertIsNotEnabled()
        composeRule.onNodeWithTag("manual-endpoint-cancel")
            .assertIsDisplayed().assertTextContains("Cancel")
        restoration.emulateSavedInstanceStateRestore()
        composeRule.onNodeWithTag("legacy-lan-security-warning").performScrollTo()
        composeRule.onNodeWithTag("manual-endpoint-cancel")
            .assertIsDisplayed().performClick()
        composeRule.runOnIdle {
            assertThat(cancelCalls).isEqualTo(1)
            state = state.copy(manualConnectState = ManualConnectUiState.Idle)
        }
        composeRule.onNodeWithTag("manual-endpoint-connect")
            .assertIsDisplayed().assertIsNotEnabled()
        composeRule.onNodeWithTag("manual-endpoint-cancel")
            .assertIsDisplayed().assertTextContains("Back")
    }
}
