package com.ekkus.silentdisco

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.core.rust.RustTrustedDevice
import com.ekkus.silentdisco.feature.settings.TrustedDevicesScreen
import com.ekkus.silentdisco.feature.settings.TrustedDevicesUiState
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import org.junit.Rule
import org.junit.Test

class TrustedDevicesAdaptiveLayoutTest {
    @get:Rule
    val composeRule = createComposeRule()

    @Test
    fun removalActionRemainsReachableAtTwoHundredPercentFontScale() {
        val device = RustTrustedDevice(
            deviceId = "adaptive-approved-phone",
            displayName = "Kitchen phone with a longer accessible name",
            lastSeenMs = 1_750_000_000_000L,
        )
        composeRule.setContent {
            CompositionLocalProvider(
                LocalDensity provides Density(density = 1f, fontScale = 2f),
            ) {
                SilentDiscoTheme {
                    Box(modifier = Modifier.size(width = 320.dp, height = 480.dp)) {
                        TrustedDevicesScreen(
                            uiState = TrustedDevicesUiState(devices = listOf(device)),
                            onBack = {},
                            onRefresh = {},
                            onDelete = {},
                        )
                    }
                }
            }
        }

        composeRule.onNodeWithTag("trusted-device-remove-adaptive-approved-phone")
            .performScrollTo()
            .assertIsDisplayed()
    }

    @Test
    fun retryActionRemainsReachableAtTwoHundredPercentFontScale() {
        composeRule.setContent {
            CompositionLocalProvider(
                LocalDensity provides Density(density = 1f, fontScale = 2f),
            ) {
                SilentDiscoTheme {
                    Box(modifier = Modifier.size(width = 320.dp, height = 480.dp)) {
                        TrustedDevicesScreen(
                            uiState = TrustedDevicesUiState(
                                error = "Approved devices could not be loaded. Try again.",
                            ),
                            onBack = {},
                            onRefresh = {},
                            onDelete = {},
                        )
                    }
                }
            }
        }

        composeRule.onNodeWithTag("trusted-devices-retry")
            .performScrollTo()
            .assertIsDisplayed()
    }
}
