package com.ekkus.silentdisco

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import com.ekkus.silentdisco.core.rust.RustTrustedDevice
import com.ekkus.silentdisco.feature.settings.TrustedDevicesScreen
import com.ekkus.silentdisco.feature.settings.TrustedDevicesUiState
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import com.google.common.truth.Truth.assertThat
import org.junit.Rule
import org.junit.Test

class TrustedDevicesScreenTest {
    @get:Rule
    val composeRule = createComposeRule()

    private val device = RustTrustedDevice(
        deviceId = "internal-device-123",
        displayName = "Kitchen phone",
        lastSeenMs = 0L,
    )

    @Test
    fun approvedDeviceCardHidesInternalIdentifier() {
        composeRule.setContent {
            SilentDiscoTheme {
                TrustedDevicesScreen(
                    uiState = TrustedDevicesUiState(devices = listOf(device)),
                    onBack = {},
                    onRefresh = {},
                    onDelete = {},
                )
            }
        }

        composeRule.onNodeWithText("Kitchen phone").assertIsDisplayed()
        composeRule.onNodeWithText("Device ID: internal-device-123").assertDoesNotExist()
        composeRule.onNodeWithTag("trusted-device-remove-internal-device-123").assertIsDisplayed()
    }

    @Test
    fun legacyRecordWhoseNameEqualsItsInternalKeyUsesGenericCopy() {
        val legacyDevice = RustTrustedDevice(
            deviceId = "legacy-internal-device-key",
            displayName = "legacy-internal-device-key",
            lastSeenMs = 0L,
        )
        composeRule.setContent {
            SilentDiscoTheme {
                TrustedDevicesScreen(
                    uiState = TrustedDevicesUiState(devices = listOf(legacyDevice)),
                    onBack = {},
                    onRefresh = {},
                    onDelete = {},
                )
            }
        }

        composeRule.onNodeWithText("Approved phone").assertIsDisplayed()
        composeRule.onNodeWithText("legacy-internal-device-key").assertDoesNotExist()
        composeRule.onNodeWithTag("trusted-device-remove-legacy-internal-device-key").performClick()
        composeRule.onNodeWithText("Approved phone will need approval before joining a future session.")
            .assertIsDisplayed()
    }

    @Test
    fun removalRequiresConfirmationAndReturnsOnlyTheInternalKey() {
        var deletedDeviceId: String? = null
        composeRule.setContent {
            SilentDiscoTheme {
                TrustedDevicesScreen(
                    uiState = TrustedDevicesUiState(devices = listOf(device)),
                    onBack = {},
                    onRefresh = {},
                    onDelete = { deletedDeviceId = it },
                )
            }
        }

        composeRule.onNodeWithTag("trusted-device-remove-internal-device-123").performClick()
        composeRule.onNodeWithTag("remove-trusted-device-confirmation").assertIsDisplayed()
        composeRule.onNodeWithText("Kitchen phone will need approval before joining a future session.")
            .assertIsDisplayed()
        composeRule.onNodeWithTag("remove-trusted-device-confirmation-destructive").performClick()

        composeRule.runOnIdle {
            assertThat(deletedDeviceId).isEqualTo("internal-device-123")
        }
    }
}
