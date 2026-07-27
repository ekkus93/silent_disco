package com.ekkus.silentdisco

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performScrollTo
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.StorageInitializationState
import com.ekkus.silentdisco.core.permissions.AppPermission
import com.ekkus.silentdisco.core.permissions.PermissionState
import com.ekkus.silentdisco.feature.settings.SettingsScreen
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import org.junit.Rule
import org.junit.Test

class SettingsScreenTest {
    @get:Rule
    val composeRule = createComposeRule()

    @Test
    fun readySettingsShowTechnicalReadinessWithoutTrustedDevicePlaceholder() {
        composeRule.setContent {
            SilentDiscoTheme {
                SettingsScreen(
                    uiState = AppUiState(
                        storageState = StorageInitializationState.READY,
                        permissions = listOf(
                            PermissionState(AppPermission.WifiState, granted = true),
                        ),
                    ),
                    trustedDeviceManagementAvailable = false,
                    onBack = {},
                    onOpenSystemSettings = {},
                    onOpenTrustedDevices = {},
                    onOpenAdvancedDiagnostics = {},
                )
            }
        }

        composeRule.onNodeWithTag("settings-storage").assertIsDisplayed()
        composeRule.onNodeWithText("Available").assertIsDisplayed()
        composeRule.onNodeWithText("Open system settings").assertIsDisplayed()
        composeRule.onNodeWithText("Advanced diagnostics")
            .performScrollTo()
            .assertIsDisplayed()
        composeRule.onNodeWithText("Manage approved devices").assertDoesNotExist()
    }

    @Test
    fun fatalStorageAndDeniedPermissionsRemainVisible() {
        composeRule.setContent {
            SilentDiscoTheme {
                SettingsScreen(
                    uiState = AppUiState(
                        storageState = StorageInitializationState.FATAL_FAILURE,
                        storageError = "database checksum mismatch",
                        permissions = listOf(
                            PermissionState(AppPermission.WifiState, granted = false),
                        ),
                    ),
                    trustedDeviceManagementAvailable = false,
                    onBack = {},
                    onOpenSystemSettings = {},
                    onOpenTrustedDevices = {},
                    onOpenAdvancedDiagnostics = {},
                )
            }
        }

        composeRule.onNodeWithText("Needs attention").assertIsDisplayed()
        composeRule.onNodeWithText("Could not be opened").assertIsDisplayed()
        composeRule.onNodeWithText("database checksum mismatch").assertDoesNotExist()
    }

    @Test
    fun approvedDeviceEntryAppearsOnlyWhenAuthoritativeManagementExists() {
        composeRule.setContent {
            SilentDiscoTheme {
                SettingsScreen(
                    uiState = AppUiState(storageState = StorageInitializationState.READY),
                    trustedDeviceManagementAvailable = true,
                    onBack = {},
                    onOpenSystemSettings = {},
                    onOpenTrustedDevices = {},
                    onOpenAdvancedDiagnostics = {},
                )
            }
        }

        composeRule.onNodeWithTag("settings-approved-devices").assertIsDisplayed()
        composeRule.onNodeWithText("Manage approved devices").assertIsDisplayed()
    }
}
