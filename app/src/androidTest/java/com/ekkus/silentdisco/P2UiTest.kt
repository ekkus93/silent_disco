package com.ekkus.silentdisco

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.P2StorageState
import com.ekkus.silentdisco.app.P2UiState
import com.ekkus.silentdisco.app.RecentAvailability
import com.ekkus.silentdisco.app.StorageInitializationState
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.rust.P2RecentSession
import com.ekkus.silentdisco.core.rust.P2SessionOutcome
import com.ekkus.silentdisco.core.rust.P2TrustedHost
import com.ekkus.silentdisco.core.rust.P2ValidatedInvitation
import com.ekkus.silentdisco.feature.home.RoleFirstHomeScreen
import com.ekkus.silentdisco.feature.listener.NearbySessionsScreen
import com.ekkus.silentdisco.feature.p2.TrustedHostsScreen
import com.ekkus.silentdisco.feature.p2.VerifiedQrInvitationDialog
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import com.google.common.truth.Truth.assertThat
import org.junit.Rule
import org.junit.Test

class P2UiTest {
    @get:Rule
    val composeRule = createComposeRule()

    @Test
    fun recentSessionNeverClaimsAvailabilityBeforeDiscovery() {
        var checks = 0
        composeRule.setContent {
            SilentDiscoTheme {
                RoleFirstHomeScreen(
                    uiState = AppUiState(storageState = StorageInitializationState.READY),
                    onHostClick = {},
                    onJoinClick = {},
                    onSettingsClick = {},
                    onRetryStorage = {},
                    recentSession = recentSession(),
                    recentAvailability = RecentAvailability.IDLE,
                    onCheckRecentSession = { checks += 1 },
                )
            }
        }

        composeRule.onNodeWithTag("home-recent-session").assertIsDisplayed()
        composeRule.onNodeWithTag("home-recent-availability")
            .performScrollTo()
            .assertTextEquals("Availability has not been checked.")
        composeRule.onNodeWithTag("home-check-recent").performScrollTo().performClick()
        composeRule.runOnIdle { assertThat(checks).isEqualTo(1) }
    }

    @Test
    fun nearbyScreenOffersSignedQrScanningSeparatelyFromDiscovery() {
        var scans = 0
        composeRule.setContent {
            SilentDiscoTheme {
                NearbySessionsScreen(
                    uiState = AppUiState(storageState = StorageInitializationState.READY),
                    permissionRequired = false,
                    onBack = {},
                    onRequestPermission = {},
                    onRefresh = {},
                    onSelectSession = {},
                    onScanQr = { scans += 1 },
                )
            }
        }

        composeRule.onNodeWithTag("nearby-scan-qr").assertIsDisplayed().performClick()
        composeRule.runOnIdle { assertThat(scans).isEqualTo(1) }
    }

    @Test
    fun trustedGroupingRequiresExactVerifiedSessionId() {
        val trustedSession = SessionInfo(
            id = "trusted-session",
            name = "Trusted Disco",
            hostDeviceName = "Verified host",
            approvalMode = ApprovalMode.MANUAL,
            inviteCodeRequired = false,
        )
        val sameNameUnverifiedSession = trustedSession.copy(
            id = "unverified-session",
            name = "Other Disco",
        )
        composeRule.setContent {
            SilentDiscoTheme {
                NearbySessionsScreen(
                    uiState = AppUiState(
                        storageState = StorageInitializationState.READY,
                        discoveredSessions = listOf(trustedSession, sameNameUnverifiedSession),
                    ),
                    permissionRequired = false,
                    onBack = {},
                    onRequestPermission = {},
                    onRefresh = {},
                    onSelectSession = {},
                    trustedVerifiedSessionIds = setOf(trustedSession.id),
                )
            }
        }

        composeRule.onNodeWithTag("nearby-trusted-hosts-heading").assertIsDisplayed()
        composeRule.onNodeWithText("Trusted host key verified").assertIsDisplayed()
        composeRule.onNodeWithText("Other nearby sessions").assertIsDisplayed()
    }

    @Test
    fun verifiedInvitationDistinguishesOneTimeJoinFromPersistedTrust() {
        var joinOnce = 0
        var trust = 0
        composeRule.setContent {
            SilentDiscoTheme {
                VerifiedQrInvitationDialog(
                    invitation = invitation(),
                    alreadyTrusted = false,
                    onDismiss = {},
                    onJoinOnce = { joinOnce += 1 },
                    onTrustAndJoin = { trust += 1 },
                )
            }
        }

        composeRule.onNodeWithText("Join once").performClick()
        composeRule.onNodeWithText("Trust host").performClick()
        composeRule.runOnIdle {
            assertThat(joinOnce).isEqualTo(1)
            assertThat(trust).isEqualTo(1)
        }
    }

    @Test
    fun trustedHostsScreenExplainsKeyBasedTrustAndDeletesByFingerprint() {
        val fingerprint = "a".repeat(64)
        var deleted: String? = null
        composeRule.setContent {
            SilentDiscoTheme {
                TrustedHostsScreen(
                    uiState = P2UiState(
                        storageState = P2StorageState.READY,
                        trustedHosts = listOf(
                            P2TrustedHost(
                                fingerprint = fingerprint,
                                displayName = "Verified host",
                                publicKeyDer = byteArrayOf(1, 2, 3),
                                lastVerifiedMs = 1L,
                            ),
                        ),
                    ),
                    onBack = {},
                    onDelete = { deleted = it },
                )
            }
        }

        composeRule.onNodeWithText("A trusted host is identified by its verified public key, not by its visible name.")
            .assertIsDisplayed()
        composeRule.onNodeWithTag("delete-trusted-host-aaaaaaaa").performClick()
        composeRule.runOnIdle { assertThat(deleted).isEqualTo(fingerprint) }
    }

    private fun recentSession() = P2RecentSession(
        sessionId = "session-1",
        sessionName = "Rooftop Disco",
        hostName = "Host phone",
        hostFingerprint = null,
        startedAtMs = 1L,
        endedAtMs = 2L,
        outcome = P2SessionOutcome.COMPLETED,
    )

    private fun invitation() = P2ValidatedInvitation(
        sessionId = "session-1",
        sessionName = "Rooftop Disco",
        hostName = "Host phone",
        hostFingerprint = "a".repeat(64),
        hostPublicKeyDer = byteArrayOf(1, 2, 3),
        approvalMode = "manual",
        inviteCode = null,
        issuedAtMs = 1L,
        expiresAtMs = 2L,
    )
}
