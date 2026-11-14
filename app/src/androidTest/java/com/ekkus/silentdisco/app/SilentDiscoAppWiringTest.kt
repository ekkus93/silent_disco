package com.ekkus.silentdisco.app

import android.app.Application
import android.net.Uri
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.semantics.SemanticsActions
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStore
import androidx.navigation.compose.ComposeNavigator
import androidx.navigation.compose.DialogNavigator
import androidx.navigation.testing.TestNavHostController
import androidx.test.core.app.ApplicationProvider
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.JoinRequest
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.model.AppRole
import com.ekkus.silentdisco.core.permissions.AppPermission
import com.ekkus.silentdisco.core.rust.FakeHostCoreController
import com.ekkus.silentdisco.core.rust.FakeListenerCoreController
import com.ekkus.silentdisco.core.rust.FakeListenerTransport
import com.ekkus.silentdisco.core.rust.FakeRustDomainStore
import com.ekkus.silentdisco.core.rust.RustDatabaseException
import com.ekkus.silentdisco.core.rust.RustTrustedDevice
import com.ekkus.silentdisco.core.transport.FakeBleTransport
import com.ekkus.silentdisco.core.transport.FakeMdnsTransport
import com.ekkus.silentdisco.core.transport.FakeSessionTransport
import com.ekkus.silentdisco.feature.settings.TrustedDeviceStore
import com.ekkus.silentdisco.feature.settings.TrustedDevicesViewModel
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CompletableDeferred
import org.junit.After
import org.junit.Rule
import org.junit.Test

/**
 * Characterization of `SilentDiscoApp`'s wiring (God-file refactor TODO §6):
 * the real app composable, on a real `MainViewModel` built over recording
 * fakes, with each destination's main callbacks tapped and their effect
 * checked as a route change or a recorded view-model/core call.
 *
 * Callbacks that leave the app (share sheets, system settings, the document
 * picker and the QR camera) are not tapped: they start other activities.
 * Delete actions are not tapped either; the trusted-hosts store is the app's
 * real one.
 */
class SilentDiscoAppWiringTest {
    @get:Rule
    val composeRule = createComposeRule()

    private val application: Application = ApplicationProvider.getApplicationContext()
    private val store = ViewModelStore()
    private val domainStore = FakeRustDomainStore()
    private val hostCore = FakeHostCoreController()
    private val listenerCore = FakeListenerCoreController()
    private val trustedDevices = RecordingTrustedDeviceStore()
    private lateinit var viewModel: MainViewModel
    private lateinit var navController: TestNavHostController

    private class RecordingTrustedDeviceStore : TrustedDeviceStore {
        var listCalls = 0
        override suspend fun initialize() = Unit
        override suspend fun listTrustedDevices(): List<RustTrustedDevice> {
            listCalls += 1
            return emptyList()
        }
        override suspend fun deleteTrustedDevice(deviceId: String) = false
        override suspend fun close() = Unit
    }

    private val factory = object : ViewModelProvider.Factory {
        @Suppress("UNCHECKED_CAST")
        override fun <T : ViewModel> create(modelClass: Class<T>): T = when (modelClass) {
            MainViewModel::class.java -> MainViewModel(
                application = application,
                domainStore = domainStore,
                hostCoreFactory = { hostCore },
                listenerCoreFactory = { listenerCore },
                bleService = FakeBleTransport(),
                wifiDirectService = FakeSessionTransport(),
                mdnsService = FakeMdnsTransport(),
                listenerTransportController = FakeListenerTransport(),
            )
            WorkflowViewModel::class.java -> WorkflowViewModel()
            TrustedDevicesViewModel::class.java -> TrustedDevicesViewModel(application, trustedDevices)
            P2ViewModel::class.java -> P2ViewModel(application)
            else -> error("unexpected view-model $modelClass")
        } as T
    }

    private fun startApp(storageFailure: Throwable? = null) {
        domainStore.initializeFailure = storageFailure
        // A real store opens after the first frame; release this one only then.
        val storageOpened = CompletableDeferred<Unit>()
        domainStore.initializeGate = storageOpened
        val provider = ViewModelProvider(store, factory)
        viewModel = provider[MainViewModel::class.java]
        val workflow = provider[WorkflowViewModel::class.java]
        val trusted = provider[TrustedDevicesViewModel::class.java]
        val p2 = provider[P2ViewModel::class.java]
        composeRule.setContent {
            val context = LocalContext.current
            val controller = remember {
                TestNavHostController(context).apply {
                    navigatorProvider.addNavigator(ComposeNavigator())
                    navigatorProvider.addNavigator(DialogNavigator())
                }
            }
            navController = controller
            SilentDiscoApp(viewModel, workflow, trusted, p2, controller)
        }
        composeRule.waitForIdle()
        storageOpened.complete(Unit)
        composeRule.runOnIdle {
            AppPermission.entries.forEach { viewModel.updatePermission(it.androidPermission, true) }
        }
    }

    @After
    fun clearViewModels() = store.clear()

    private fun route(): String? = composeRule.runOnIdle { navController.currentBackStackEntry?.destination?.route }

    private fun awaitRoute(expected: String) {
        try {
            composeRule.waitUntil(5_000) { route() == expected }
        } catch (timeout: androidx.compose.ui.test.ComposeTimeoutException) {
            throw AssertionError("expected route $expected but was ${route()}", timeout)
        }
    }

    private fun go(destination: String) {
        composeRule.runOnIdle { navController.navigateSingleTop(destination) }
        awaitRoute(destination)
    }

    private fun updateUi(transform: (AppUiState) -> AppUiState) {
        composeRule.runOnIdle { viewModel._uiState.value = transform(viewModel._uiState.value) }
        composeRule.waitForIdle()
    }

    /** Scrolls to [this] when it sits in a scrollable container; many screens fit without one. */
    private fun SemanticsNodeInteraction.scrolledIntoView(): SemanticsNodeInteraction = apply {
        try {
            performScrollTo()
        } catch (_: AssertionError) {
            // No scrollable parent: the node is already laid out on screen.
        }
    }

    /**
     * Runs the node's click action directly. These tests pin wiring, not
     * touch hit-testing: a snackbar can briefly cover bottom-anchored buttons.
     */
    private fun SemanticsNodeInteraction.invokeClick(): SemanticsNodeInteraction =
        performSemanticsAction(SemanticsActions.OnClick)

    private fun clickTag(tag: String) = composeRule.onNodeWithTag(tag).scrolledIntoView().invokeClick()
    private fun clickBack() = composeRule.onNodeWithContentDescription("Back").invokeClick()

    private val song = SelectedAudioFile(
        uri = Uri.parse("content://silent-disco-test/song-1"),
        displayName = "Song one.wav",
        mimeType = "audio/wav",
        sizeBytes = 1_000L,
    )
    private val session = SessionInfo(
        id = "session-1",
        name = "Patio",
        hostDeviceName = "Host phone",
        approvalMode = ApprovalMode.MANUAL,
        inviteCodeRequired = false,
    )

    private fun startHome() {
        startApp()
        awaitRoute(AppRoutes.Home)
    }

    @Test
    fun startupRetryReopensStorageAndReachesHomeWhenReady() {
        startApp(storageFailure = RustDatabaseException(-105, "open database"))
        composeRule.waitUntil(5_000) {
            viewModel.uiState.value.storageState == StorageInitializationState.RECOVERABLE_FAILURE
        }
        assertThat(route()).isEqualTo(AppRoutes.Startup)
        val opened = domainStore.initializeCallCount
        domainStore.initializeFailure = null

        clickTag("startup-retry")

        awaitRoute(AppRoutes.Home)
        assertThat(domainStore.initializeCallCount).isEqualTo(opened + 1)
    }

    @Test
    fun homeOpensHostSetupNearbySessionsAndSettings() {
        startHome()
        composeRule.onNodeWithTag("home-host").invokeClick()
        awaitRoute(AppRoutes.HostMusicSetup)
        assertThat(viewModel.uiState.value.selectedRole).isEqualTo(AppRole.HOST)

        clickBack()
        awaitRoute(AppRoutes.Home)
        composeRule.onNodeWithTag("home-join").invokeClick()
        awaitRoute(AppRoutes.NearbySessions)
        assertThat(viewModel.uiState.value.selectedRole).isEqualTo(AppRole.LISTENER)

        go(AppRoutes.Home)
        composeRule.onNodeWithTag("home-settings").invokeClick()
        awaitRoute(AppRoutes.Settings)
    }

    @Test
    fun hostSetupWalksMusicAccessAndReadyAndStartStagesTheSongs() {
        startHome()
        go(AppRoutes.HostMusicSetup)
        updateUi { it.copy(hostForm = it.hostForm.copy(selectedAudio = song)) }
        clickTag("host-music-next")
        awaitRoute(AppRoutes.HostAccessSetup)

        clickTag("host-access-invite_code")
        clickTag("host-generate-code")
        composeRule.waitUntil(5_000) { viewModel.uiState.value.hostForm.inviteCode.matches(Regex("\\d{4}")) }
        clickTag("host-access-manual")
        clickTag("host-access-next")
        awaitRoute(AppRoutes.HostReady)

        clickBack()
        awaitRoute(AppRoutes.HostAccessSetup)
        clickTag("host-access-next")
        awaitRoute(AppRoutes.HostReady)
        // Start stages the chosen song first. This test's song is not a real
        // provider file, so staging fails visibly before the core is asked.
        clickTag("host-start-session")
        composeRule.waitUntil(5_000) { viewModel.uiState.value.lastError?.contains("Song one.wav") == true }
        assertThat(hostCore.configureAndCreateCalls).isEmpty()

        go(AppRoutes.HostMusicSetup)
        clickBack()
        assertThat(route()).isNotEqualTo(AppRoutes.HostMusicSetup)
    }

    @Test
    fun hostDashboardApprovesOpensQueueAndHelpAndAsksBeforeEnding() {
        startHome()
        viewModel.ensureRustHostCore()
        // ADVERTISING makes the app itself open the dashboard (the workflow's host navigation).
        updateUi {
            it.copy(
                hostState = HostLifecycleState.ADVERTISING,
                pendingJoinRequests = listOf(
                    JoinRequest("req-1", "session-1", "listener-1", "Alex", null, 0L),
                ),
            )
        }
        awaitRoute(AppRoutes.HostDashboard)
        composeRule.onNodeWithTag("host-dashboard-list").performScrollToNode(hasText("Approve once"))
        composeRule.onNodeWithText("Approve once").assertIsEnabled().invokeClick()
        composeRule.waitUntil(5_000) { hostCore.approveJoinCalls.isNotEmpty() }
        assertThat(hostCore.approveJoinCalls.single()).isEqualTo("req-1" to false)

        composeRule.onNodeWithContentDescription("Manage queue").invokeClick()
        awaitRoute(AppRoutes.HostQueueManagement)
        clickBack()
        awaitRoute(AppRoutes.HostDashboard)

        composeRule.onNodeWithTag("host-session-actions").invokeClick()
        composeRule.onNodeWithText("End session").invokeClick()
        composeRule.onNodeWithText("End this session?").assertExists()
        composeRule.onNodeWithTag("end-session-confirmation-safe").invokeClick()
        assertThat(route()).isEqualTo(AppRoutes.HostDashboard)

        clickBack()
        composeRule.onNodeWithText("End this session?").assertExists()
    }

    @Test
    fun hostQueueManagementRemovesAnUpcomingSong() {
        startHome()
        viewModel.ensureRustHostCore()
        go(AppRoutes.HostDashboard)
        go(AppRoutes.HostQueueManagement)
        updateUi {
            it.copy(
                hostQueue = listOf(
                    HostQueueItem("first", "s1", "First song", 1L, null),
                    HostQueueItem("second", "s2", "Second song", 1L, null),
                ),
            )
        }
        clickTag("host-queue-remove-second")
        composeRule.waitUntil(5_000) { hostCore.removeHostQueueEntryCalls.isNotEmpty() }
        assertThat(hostCore.removeHostQueueEntryCalls).containsExactly("second")

        clickBack()
        awaitRoute(AppRoutes.HostDashboard)
    }

    @Test
    fun nearbySessionsScansAndOpensManualConnectAndAJoin() {
        startHome()
        go(AppRoutes.NearbySessions)
        composeRule.waitUntil(5_000) { listenerCore.startDiscoveryCallCount > 0 }

        clickTag("nearby-connect-manually")
        awaitRoute(AppRoutes.ManualConnect)
        composeRule.onNodeWithContentDescription("Cancel and go back").invokeClick()
        awaitRoute(AppRoutes.NearbySessions)

        updateUi { it.copy(discoveredSessions = listOf(session)) }
        composeRule.onNodeWithText("Join").scrolledIntoView().invokeClick()
        awaitRoute(AppRoutes.SessionJoin)
        assertThat(listenerCore.selectSessionCalls).contains("session-1")
    }

    @Test
    fun sessionJoinRequestsTheJoinAndCancelReturnsToNearbySessions() {
        startHome()
        go(AppRoutes.NearbySessions)
        go(AppRoutes.SessionJoin)
        updateUi {
            it.copy(
                selectedRole = AppRole.LISTENER,
                discoveredSessions = listOf(session),
                selectedSession = session,
                listenerState = ListenerLifecycleState.SESSION_SELECTED,
            )
        }
        clickTag("session-request-join")
        composeRule.waitUntil(5_000) { listenerCore.submitJoinCalls.isNotEmpty() }

        composeRule.onNodeWithContentDescription("Cancel and go back").invokeClick()
        awaitRoute(AppRoutes.NearbySessions)
        assertThat(listenerCore.cancelJoinCallCount).isAtLeast(1)
    }

    @Test
    fun listenerPlaybackSetsVolumeOpensHelpAndAsksBeforeLeaving() {
        startHome()
        go(AppRoutes.ListenerPlayback)
        updateUi {
            it.copy(
                selectedRole = AppRole.LISTENER,
                selectedSession = session,
                listenerState = ListenerLifecycleState.DESYNCED,
            )
        }
        composeRule.onNode(
            SemanticsMatcher.keyIsDefined(SemanticsActions.SetProgress),
            useUnmergedTree = true,
        ).performSemanticsAction(SemanticsActions.SetProgress) { it(0.25f) }
        composeRule.waitUntil(5_000) { viewModel.uiState.value.localVolume == 0.25f }

        clickTag("listener-leave")
        composeRule.onNodeWithText("Leave this session?").assertExists()
        composeRule.onNodeWithTag("leave-session-confirmation-safe").invokeClick()

        clickTag("listener-fix-connection")
        awaitRoute(AppRoutes.ConnectionHelp)
    }

    @Test
    fun connectionHelpReconnectsAndOpensAdvancedDiagnostics() {
        startHome()
        viewModel.ensureRustListenerCore()
        go(AppRoutes.ConnectionHelp)
        updateUi {
            it.copy(
                selectedRole = AppRole.LISTENER,
                selectedSession = session,
                listenerState = ListenerLifecycleState.DISCONNECTED,
            )
        }
        clickTag("connection-help-advanced")
        awaitRoute(AppRoutes.AdvancedDiagnostics)
        clickBack()
        awaitRoute(AppRoutes.ConnectionHelp)

        clickTag("connection-help-reconnect")
        awaitRoute(AppRoutes.NearbySessions)
        assertThat(listenerCore.retryRecoverableFailureCallCount).isAtLeast(1)
    }

    @Test
    fun settingsOpensTrustedDevicesTrustedHostsAndDiagnostics() {
        startHome()
        go(AppRoutes.Settings)
        composeRule.onNodeWithText("Manage approved devices").scrolledIntoView().invokeClick()
        awaitRoute(AppRoutes.TrustedDevices)
        composeRule.waitUntil(5_000) { trustedDevices.listCalls > 0 }
        clickBack()
        awaitRoute(AppRoutes.Settings)

        composeRule.onNodeWithText("Manage trusted hosts").scrolledIntoView().invokeClick()
        awaitRoute(AppRoutes.TrustedHosts)
        clickBack()
        awaitRoute(AppRoutes.Settings)

        composeRule.onNodeWithText("Advanced diagnostics").scrolledIntoView().invokeClick()
        awaitRoute(AppRoutes.AdvancedDiagnostics)
        clickBack()
        awaitRoute(AppRoutes.Settings)
        clickBack()
        awaitRoute(AppRoutes.Home)
    }

    @Test
    fun manualConnectTypesAPayloadIntoTheForm() {
        startHome()
        go(AppRoutes.NearbySessions)
        go(AppRoutes.ManualConnect)
        composeRule.onNodeWithTag("manual-endpoint-input").performTextInput("{}")
        composeRule.waitUntil(5_000) { viewModel.uiState.value.manualEndpointForm.rawInput == "{}" }
        composeRule.onNodeWithTag("manual-endpoint-cancel").scrolledIntoView().invokeClick()
        awaitRoute(AppRoutes.NearbySessions)
    }
}
