package com.ekkus.silentdisco.feature.listener

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsNotDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performScrollTo
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.core.model.ManualConnectUiState
import com.ekkus.silentdisco.core.model.ManualEndpointFormState
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import org.junit.Rule
import org.junit.Test

class ManualEndpointScreenTest {
    @get:Rule
    val composeRule = createComposeRule()

    private fun setContent(uiState: AppUiState) {
        composeRule.setContent {
            SilentDiscoTheme {
                ManualEndpointScreen(
                    uiState = uiState,
                    onBack = {},
                    onInputChanged = {},
                    onInviteCodeChanged = {},
                    onConnect = {},
                    onCancel = {},
                )
            }
        }
    }

    @Test
    fun connectIsDisabledUntilPayloadIsValid() {
        setContent(AppUiState(manualEndpointForm = ManualEndpointFormState()))

        composeRule.onNodeWithTag("manual-endpoint-connect").assertIsNotEnabled()
        composeRule.onNodeWithTag("manual-endpoint-preview").assertIsNotDisplayed()
    }

    @Test
    fun validPayloadShowsPreviewAndEnablesConnect() {
        setContent(
            AppUiState(
                manualEndpointForm = ManualEndpointFormState(
                    rawInput = "{\"hostAddress\":\"192.168.1.50\"}",
                    hostAddress = "192.168.1.50",
                    sessionId = "session-1",
                    protocolVersion = 1,
                ),
            ),
        )

        composeRule.onNodeWithTag("manual-endpoint-preview").assertIsDisplayed()
        composeRule.onNodeWithText("Host: 192.168.1.50").assertIsDisplayed()
        composeRule.onNodeWithText("Protocol version: 1").assertDoesNotExist()
        composeRule.onNodeWithTag("manual-endpoint-connect").assertIsEnabled()
    }

    @Test
    fun aV3HostSaysTheQrIsRequiredAndShowsNoSecureNotice() {
        setContent(
            AppUiState(
                manualEndpointForm = ManualEndpointFormState(
                    rawInput = "{}",
                    hostAddress = "192.168.1.50",
                    sessionId = "session-1",
                    protocolVersion = 3,
                ),
            ),
        )

        composeRule.onNodeWithTag("qr-required-notice").assertExists()
        composeRule.onNodeWithTag("secure-transport-notice").assertDoesNotExist()
    }

    @Test
    fun aSecureTransportHostShowsTheSecureNoticeInsteadOfTheQrRequiredNotice() {
        setContent(
            AppUiState(
                manualEndpointForm = ManualEndpointFormState(
                    rawInput = "{}",
                    hostAddress = "192.168.1.50",
                    sessionId = "session-1",
                    protocolVersion = 4,
                    secureTransport = true,
                ),
            ),
        )

        composeRule.onNodeWithTag("secure-transport-notice").assertExists()
        composeRule.onNodeWithText("Secure connection (v4, experimental)").assertExists()
        composeRule.onNodeWithTag("qr-required-notice").assertDoesNotExist()
        composeRule.onNodeWithText("Scan the host's QR code to join").assertDoesNotExist()
    }

    @Test
    fun validationErrorIsShownNextToTheField() {
        setContent(
            AppUiState(
                manualEndpointForm = ManualEndpointFormState(
                    rawInput = "not json",
                    validationError = "connection payload is not valid JSON",
                ),
            ),
        )

        composeRule.onNodeWithText("connection payload is not valid JSON").assertIsDisplayed()
        composeRule.onNodeWithTag("manual-endpoint-connect").assertIsNotEnabled()
    }

    @Test
    fun inviteCodeRequiredBlocksConnectUntilEntered() {
        setContent(
            AppUiState(
                manualEndpointForm = ManualEndpointFormState(
                    hostAddress = "192.168.1.50",
                    sessionId = "session-1",
                    inviteCodeRequired = true,
                ),
            ),
        )

        composeRule.onNodeWithTag("manual-endpoint-invite-code").performScrollTo().assertIsDisplayed()
        composeRule.onNodeWithTag("manual-endpoint-connect").assertIsNotEnabled()
    }

    @Test
    fun connectingShowsProgressAndHidesConnectButton() {
        setContent(
            AppUiState(
                manualEndpointForm = ManualEndpointFormState(
                    hostAddress = "192.168.1.50",
                    sessionId = "session-1",
                ),
                manualConnectState = ManualConnectUiState.Connecting,
            ),
        )

        composeRule.onNodeWithTag("manual-endpoint-progress").assertIsDisplayed()
        composeRule.onNodeWithTag("manual-endpoint-connect").assertIsNotDisplayed()
        composeRule.onNodeWithTag("manual-endpoint-cancel").assertIsDisplayed()
    }

    @Test
    fun awaitingApprovalShowsHostAndSessionName() {
        setContent(
            AppUiState(
                manualConnectState = ManualConnectUiState.AwaitingApproval(
                    hostName = "Desktop Host",
                    sessionName = "Back Patio",
                ),
            ),
        )

        composeRule.onNodeWithText(
            "Waiting for Desktop Host to approve \"Back Patio\"…",
        ).assertIsDisplayed()
    }

    @Test
    fun approvedStateShowsConfirmationWithoutCancelOrConnect() {
        setContent(
            AppUiState(manualConnectState = ManualConnectUiState.Approved(trustedForFuture = true)),
        )

        composeRule.onNodeWithTag("manual-endpoint-approved").assertIsDisplayed()
        composeRule.onNodeWithTag("manual-endpoint-connect").assertIsNotDisplayed()
        composeRule.onNodeWithTag("manual-endpoint-cancel").assertIsNotDisplayed()
    }

    @Test
    fun rejectionShowsReasonWithRetryAction() {
        setContent(
            AppUiState(manualConnectState = ManualConnectUiState.Rejected("invite_code_invalid")),
        )

        composeRule.onNodeWithTag("manual-endpoint-problem").assertIsDisplayed()
        composeRule.onNodeWithText("invite_code_invalid").assertIsDisplayed()
    }
}
