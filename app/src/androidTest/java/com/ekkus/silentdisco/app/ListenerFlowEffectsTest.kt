package com.ekkus.silentdisco.app

import android.app.Application
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStore
import androidx.navigation.compose.ComposeNavigator
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.testing.TestNavHostController
import androidx.test.core.app.ApplicationProvider
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.rust.FakeHostCoreController
import com.ekkus.silentdisco.core.rust.FakeListenerCoreController
import com.ekkus.silentdisco.core.rust.FakeListenerTransport
import com.ekkus.silentdisco.core.rust.FakeRustDomainStore
import com.ekkus.silentdisco.core.rust.P2RecentSession
import com.ekkus.silentdisco.core.rust.P2SessionOutcome
import com.ekkus.silentdisco.core.rust.P2ValidatedInvitation
import com.ekkus.silentdisco.core.transport.FakeBleTransport
import com.ekkus.silentdisco.core.transport.FakeMdnsTransport
import com.ekkus.silentdisco.core.transport.FakeSessionTransport
import com.google.common.truth.Truth.assertThat
import org.junit.After
import org.junit.Rule
import org.junit.Test

/**
 * The listener flow effects moved out of `SilentDiscoApp` (God-file refactor
 * TODO §6.5): the verified-invitation dialog flag, the recent-session rejoin
 * and the QR join that waits for its discovery scan.
 */
class ListenerFlowEffectsTest {
    @get:Rule
    val composeRule = createComposeRule()

    private val application: Application = ApplicationProvider.getApplicationContext()
    private val store = ViewModelStore()
    private val listenerCore = FakeListenerCoreController()
    private val qrJoin = QrJoinFlowState()
    private val snackbar = SnackbarHostState()
    private lateinit var viewModel: MainViewModel
    private lateinit var navController: TestNavHostController
    private var uiState by mutableStateOf(AppUiState())
    private var p2UiState by mutableStateOf(P2UiState())

    private val factory = object : ViewModelProvider.Factory {
        @Suppress("UNCHECKED_CAST")
        override fun <T : ViewModel> create(modelClass: Class<T>): T = when (modelClass) {
            MainViewModel::class.java -> MainViewModel(
                application = application,
                domainStore = FakeRustDomainStore(),
                hostCoreFactory = { FakeHostCoreController() },
                listenerCoreFactory = { listenerCore },
                bleService = FakeBleTransport(),
                wifiDirectService = FakeSessionTransport(),
                mdnsService = FakeMdnsTransport(),
                listenerTransportController = FakeListenerTransport(),
            )
            P2ViewModel::class.java -> P2ViewModel(application)
            else -> error("unexpected view-model $modelClass")
        } as T
    }

    private val session = SessionInfo(
        id = "session-1",
        name = "Patio",
        hostDeviceName = "Host phone",
        approvalMode = ApprovalMode.MANUAL,
        inviteCodeRequired = false,
    )
    private val invitation = P2ValidatedInvitation(
        sessionId = "session-1",
        sessionName = "Patio",
        hostName = "Host phone",
        hostFingerprint = "aaaaaaaa",
        hostPublicKeyDer = byteArrayOf(1),
        approvalMode = "manual",
        inviteCode = "1234",
        issuedAtMs = 0L,
        expiresAtMs = 60_000L,
        connectionPayloadJson = null,
    )

    private fun start() {
        val provider = ViewModelProvider(store, factory)
        viewModel = provider[MainViewModel::class.java]
        val p2 = provider[P2ViewModel::class.java]
        composeRule.setContent {
            val context = LocalContext.current
            val controller = remember {
                TestNavHostController(context).apply { navigatorProvider.addNavigator(ComposeNavigator()) }
            }
            navController = controller
            NavHost(controller, startDestination = AppRoutes.NearbySessions) {
                composable(AppRoutes.NearbySessions) {}
                composable(AppRoutes.SessionJoin) {}
            }
            ListenerFlowEffects(uiState, p2UiState, qrJoin, viewModel, p2, controller, snackbar)
        }
        composeRule.waitForIdle()
        viewModel.ensureRustListenerCore()
    }

    @After
    fun clearViewModels() = store.clear()

    private fun route(): String? = composeRule.runOnIdle { navController.currentBackStackEntry?.destination?.route }

    @Test
    fun aValidatedInvitationOpensTheVerifiedDialogAndItsDismissalClosesIt() {
        start()
        composeRule.runOnIdle { p2UiState = p2UiState.copy(validatedInvitation = invitation) }
        composeRule.runOnIdle { assertThat(qrJoin.showVerifiedQrDialog).isTrue() }
        composeRule.runOnIdle { p2UiState = p2UiState.copy(validatedInvitation = null) }
        composeRule.runOnIdle { assertThat(qrJoin.showVerifiedQrDialog).isFalse() }
    }

    @Test
    fun aRecentSessionSeenNearbyAgainIsSelectedAndOpened() {
        start()
        composeRule.runOnIdle {
            p2UiState = p2UiState.copy(
                rejoinTarget = P2RecentSession("session-1", "Patio", "Host phone", null, 0L, 1L, P2SessionOutcome.COMPLETED),
                recentAvailability = RecentAvailability.AVAILABLE,
            )
        }
        assertThat(route()).isEqualTo(AppRoutes.NearbySessions)

        composeRule.runOnIdle { uiState = uiState.copy(discoveredSessions = listOf(session)) }
        composeRule.waitUntil(5_000) { route() == AppRoutes.SessionJoin }
        assertThat(listenerCore.selectSessionCalls).containsExactly("session-1")
    }

    @Test
    fun aQrJoinWaitsForItsScanThenOpensTheSessionWithTheInvitationsCode() {
        start()
        composeRule.runOnIdle {
            p2UiState = p2UiState.copy(validatedInvitation = invitation)
            qrJoin.pendingQrJoin = true
        }
        // The session is listed before the scan runs: not yet.
        composeRule.runOnIdle { uiState = uiState.copy(discoveredSessions = listOf(session)) }
        assertThat(route()).isEqualTo(AppRoutes.NearbySessions)

        composeRule.runOnIdle { uiState = uiState.copy(isScanning = true) }
        composeRule.runOnIdle { assertThat(qrJoin.qrJoinScanObserved).isTrue() }
        composeRule.runOnIdle { uiState = uiState.copy(isScanning = false) }

        composeRule.waitUntil(5_000) { route() == AppRoutes.SessionJoin }
        composeRule.runOnIdle {
            assertThat(qrJoin.pendingQrJoin).isFalse()
            assertThat(listenerCore.selectSessionCalls).containsExactly("session-1")
            assertThat(viewModel.uiState.value.connectionProgress.inviteCode).isEqualTo("1234")
        }
    }

    @Test
    fun aQrJoinWhoseSessionIsNotNearbyEndsWithAMessage() {
        start()
        composeRule.runOnIdle {
            p2UiState = p2UiState.copy(validatedInvitation = invitation)
            qrJoin.pendingQrJoin = true
        }
        composeRule.runOnIdle { uiState = uiState.copy(isScanning = true) }
        composeRule.runOnIdle { uiState = uiState.copy(isScanning = false) }

        var shown: String? = null
        composeRule.waitUntil(5_000) {
            shown = snackbar.currentSnackbarData?.visuals?.message
            shown != null
        }
        assertThat(shown).contains("not currently nearby")
        composeRule.runOnIdle {
            assertThat(qrJoin.pendingQrJoin).isFalse()
            assertThat(listenerCore.selectSessionCalls).isEmpty()
        }
        assertThat(route()).isEqualTo(AppRoutes.NearbySessions)
    }
}
