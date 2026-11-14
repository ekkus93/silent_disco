package com.ekkus.silentdisco.app

import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.createComposeRule
import com.google.common.truth.Truth.assertThat
import org.junit.Rule
import org.junit.Test

/**
 * [SessionConfirmationState] keeps every flag across a saved-instance-state restore (rotation
 * and process death), each one independently, as the separate
 * `rememberSaveable` flags it replaced did (God-file refactor TODO §6.4).
 */
class SessionConfirmationStateRestorationTest {
    @get:Rule val composeRule = createComposeRule()

    private fun flags(current: SessionConfirmationState) = mapOf(
            "showEndSessionConfirmation" to current.showEndSessionConfirmation,
            "showLeaveSessionConfirmation" to current.showLeaveSessionConfirmation,
            "showInviteDialog" to current.showInviteDialog,
    )

    @Test
    fun everyFlagStartsClearedAndSurvivesRestorationOnItsOwn() {
        lateinit var current: SessionConfirmationState
        val restoration = StateRestorationTester(composeRule)
        restoration.setContent { current = rememberSessionConfirmationState() }
        composeRule.runOnIdle { assertThat(flags(current).values).containsExactly(false, false, false) }

        for (name in flags(current).keys) {
            composeRule.runOnIdle {
                flags(current).keys.forEach { clear(current, it) }
                when (name) {
                    "showEndSessionConfirmation" -> current.showEndSessionConfirmation = true
                    "showLeaveSessionConfirmation" -> current.showLeaveSessionConfirmation = true
                    "showInviteDialog" -> current.showInviteDialog = true
                }
            }
            restoration.emulateSavedInstanceStateRestore()
            composeRule.runOnIdle {
                assertThat(flags(current).filterValues { it }.keys).containsExactly(name)
            }
        }
    }

    private fun clear(current: SessionConfirmationState, name: String) {
        when (name) {
            "showEndSessionConfirmation" -> current.showEndSessionConfirmation = false
            "showLeaveSessionConfirmation" -> current.showLeaveSessionConfirmation = false
            "showInviteDialog" -> current.showInviteDialog = false
        }
    }
}
