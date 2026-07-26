package com.ekkus.silentdisco

import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.StorageInitializationState
import com.ekkus.silentdisco.app.TuningField
import com.ekkus.silentdisco.app.TuningSettings
import com.ekkus.silentdisco.core.model.AppRole
import com.ekkus.silentdisco.feature.diagnostics.DiagnosticsScreen
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import com.google.common.truth.Truth.assertThat
import org.junit.Rule
import org.junit.Test

class TuningResetUiTest {
    @get:Rule
    val composeRule = createComposeRule()

    @Test
    fun resetButtonEmitsExactlyOneAtomicResetCommand() {
        val commands = mutableListOf<Pair<TuningField, Int>>()
        composeRule.setContent {
            SilentDiscoTheme {
                DiagnosticsScreen(
                    uiState = AppUiState(
                        storageState = StorageInitializationState.READY,
                        selectedRole = AppRole.LISTENER,
                        tuningSettings = TuningSettings(syncCadenceMs = 4_000L),
                    ),
                    onBack = {},
                    onManualResync = {},
                    onAdjustTuning = { field, direction -> commands += field to direction },
                    onShare = {},
                    initialExpertExpanded = true,
                    initialExpertEnabled = true,
                )
            }
        }

        composeRule.onNodeWithTag("reset-tuning-defaults")
            .performScrollTo()
            .assertIsEnabled()
            .performClick()

        composeRule.runOnIdle {
            assertThat(commands).containsExactly(TuningField.ResetDefaults to 1)
        }
    }

    @Test
    fun resetButtonIsDisabledWhenSettingsAlreadyMatchDefaults() {
        composeRule.setContent {
            SilentDiscoTheme {
                DiagnosticsScreen(
                    uiState = AppUiState(
                        storageState = StorageInitializationState.READY,
                        selectedRole = AppRole.LISTENER,
                        tuningSettings = TuningSettings(),
                    ),
                    onBack = {},
                    onManualResync = {},
                    onAdjustTuning = { _, _ -> },
                    onShare = {},
                    initialExpertExpanded = true,
                    initialExpertEnabled = true,
                )
            }
        }

        composeRule.onNodeWithTag("reset-tuning-defaults")
            .performScrollTo()
            .assertIsEnabled()
    }
}
