package com.ekkus.silentdisco.app

import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.createComposeRule
import com.google.common.truth.Truth.assertThat
import org.junit.Rule
import org.junit.Test

/**
 * [QrJoinFlowState] keeps every flag across a saved-instance-state restore (rotation
 * and process death), each one independently, as the separate
 * `rememberSaveable` flags it replaced did (God-file refactor TODO §6.4).
 */
class QrJoinFlowStateRestorationTest {
    @get:Rule val composeRule = createComposeRule()

    private fun flags(current: QrJoinFlowState) = mapOf(
            "pendingQrJoin" to current.pendingQrJoin,
            "qrJoinScanObserved" to current.qrJoinScanObserved,
            "showCameraRationale" to current.showCameraRationale,
            "showCameraDenied" to current.showCameraDenied,
            "showVerifiedQrDialog" to current.showVerifiedQrDialog,
    )

    @Test
    fun everyFlagStartsClearedAndSurvivesRestorationOnItsOwn() {
        lateinit var current: QrJoinFlowState
        val restoration = StateRestorationTester(composeRule)
        restoration.setContent { current = rememberQrJoinFlowState() }
        composeRule.runOnIdle { assertThat(flags(current).values).containsExactly(false, false, false, false, false) }

        for (name in flags(current).keys) {
            composeRule.runOnIdle {
                flags(current).keys.forEach { clear(current, it) }
                when (name) {
                    "pendingQrJoin" -> current.pendingQrJoin = true
                    "qrJoinScanObserved" -> current.qrJoinScanObserved = true
                    "showCameraRationale" -> current.showCameraRationale = true
                    "showCameraDenied" -> current.showCameraDenied = true
                    "showVerifiedQrDialog" -> current.showVerifiedQrDialog = true
                }
            }
            restoration.emulateSavedInstanceStateRestore()
            composeRule.runOnIdle {
                assertThat(flags(current).filterValues { it }.keys).containsExactly(name)
            }
        }
    }

    private fun clear(current: QrJoinFlowState, name: String) {
        when (name) {
            "pendingQrJoin" -> current.pendingQrJoin = false
            "qrJoinScanObserved" -> current.qrJoinScanObserved = false
            "showCameraRationale" -> current.showCameraRationale = false
            "showCameraDenied" -> current.showCameraDenied = false
            "showVerifiedQrDialog" -> current.showVerifiedQrDialog = false
        }
    }
}
