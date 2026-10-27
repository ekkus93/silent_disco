package com.ekkus.silentdisco

import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.createComposeRule
import com.ekkus.silentdisco.app.PermissionAction
import com.ekkus.silentdisco.app.PermissionRequestState
import com.ekkus.silentdisco.app.rememberPermissionRequestState
import com.google.common.truth.Truth.assertThat
import org.junit.Rule
import org.junit.Test

class PermissionRequestStateRestorationTest {
    @get:Rule val composeRule = createComposeRule()

    @Test
    fun pendingQrPermissionSurvivesRecreationAndGrantedContinuationRunsOnlyOnce() {
        lateinit var current: PermissionRequestState
        val restoration = StateRestorationTester(composeRule)
        restoration.setContent { current = rememberPermissionRequestState() }

        composeRule.runOnIdle {
            current.request(PermissionAction.QR_LISTENER_DISCOVERY)
            current.markLaunched()
        }
        restoration.emulateSavedInstanceStateRestore()
        composeRule.runOnIdle {
            assertThat(current.pendingAction).isEqualTo(PermissionAction.QR_LISTENER_DISCOVERY)
            assertThat(current.requestLaunched).isTrue()
            current.complete(granted = true)
        }
        restoration.emulateSavedInstanceStateRestore()
        composeRule.runOnIdle {
            assertThat(current.pendingAction).isNull()
            assertThat(current.requestLaunched).isFalse()
            assertThat(current.consumeGranted()).isEqualTo(PermissionAction.QR_LISTENER_DISCOVERY)
            assertThat(current.consumeGranted()).isNull()
        }
    }

    @Test
    fun deniedOrDismissedPermissionCannotNavigateAfterRestoration() {
        lateinit var current: PermissionRequestState
        val restoration = StateRestorationTester(composeRule)
        restoration.setContent { current = rememberPermissionRequestState() }

        composeRule.runOnIdle {
            current.request(PermissionAction.DIRECT_QR_CONNECT)
            current.markLaunched()
            current.complete(granted = false)
        }
        restoration.emulateSavedInstanceStateRestore()
        composeRule.runOnIdle {
            assertThat(current.pendingAction).isNull()
            assertThat(current.consumeGranted()).isNull()
            current.request(PermissionAction.HOST_SETUP)
            current.dismiss()
        }
        restoration.emulateSavedInstanceStateRestore()
        composeRule.runOnIdle {
            assertThat(current.pendingAction).isNull()
            assertThat(current.consumeGranted()).isNull()
        }
    }

    @Test
    fun audioPickerActionsKeepSetupAndLiveQueueDestinationsDistinct() {
        assertThat(PermissionAction.PICK_SETUP_AUDIO)
            .isNotEqualTo(PermissionAction.PICK_LIVE_AUDIO)
        assertThat(PermissionAction.PICK_SETUP_AUDIO.context)
            .isEqualTo(PermissionAction.PICK_LIVE_AUDIO.context)
    }
}
