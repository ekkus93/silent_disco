package com.ekkus.silentdisco

import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import com.ekkus.silentdisco.app.AppRoutes
import com.ekkus.silentdisco.app.AppUiEffect
import com.ekkus.silentdisco.app.navigateHomeAndClearWorkflow
import com.ekkus.silentdisco.app.navigateSingleTop
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.withContext
import org.junit.Rule
import org.junit.Test

class WorkflowEffectNavigationIntegrationTest {
    @get:Rule
    val composeRule = createComposeRule()

    @Test
    fun navigateHomeEffectClearsTheActiveWorkflowStack() {
        val holder = FakeWorkflowEffectHolder()
        lateinit var navController: NavHostController
        composeRule.setContent {
            SilentDiscoTheme {
                WorkflowEffectHarness(holder) { navController = it }
            }
        }

        holder.emit(AppUiEffect.NavigateHostDashboard)
        composeRule.onNodeWithTag("effect-route-host-dashboard").assertIsDisplayed()
        composeRule.waitForIdle()

        holder.emit(AppUiEffect.NavigateHome)
        composeRule.onNodeWithTag("effect-route-home").assertIsDisplayed()
        composeRule.runOnIdle {
            check(!navController.popBackStack()) {
                "Home should be the only destination after clearing the workflow"
            }
        }
        composeRule.onNodeWithTag("effect-route-home").assertIsDisplayed()
    }

    @Test
    fun repeatedHostDashboardEffectUsesSingleTopNavigation() {
        val holder = FakeWorkflowEffectHolder()
        lateinit var navController: NavHostController
        composeRule.setContent {
            SilentDiscoTheme {
                WorkflowEffectHarness(holder) { navController = it }
            }
        }

        holder.emit(AppUiEffect.NavigateHostDashboard)
        holder.emit(AppUiEffect.NavigateHostDashboard)
        composeRule.onNodeWithTag("effect-route-host-dashboard").assertIsDisplayed()

        composeRule.runOnIdle {
            check(navController.popBackStack()) { "Expected startup below the dashboard" }
        }
        composeRule.onNodeWithTag("effect-route-startup").assertIsDisplayed()
    }

    @Test
    fun listenerPlaybackEffectNavigatesToPlayback() {
        val holder = FakeWorkflowEffectHolder()
        composeRule.setContent {
            SilentDiscoTheme {
                WorkflowEffectHarness(holder)
            }
        }

        holder.emit(AppUiEffect.NavigateListenerPlayback)
        composeRule.onNodeWithTag("effect-route-listener-playback").assertIsDisplayed()
    }

    @Test
    fun confirmationEffectsRemainOnTheCurrentDestination() {
        val holder = FakeWorkflowEffectHolder()
        composeRule.setContent {
            SilentDiscoTheme {
                WorkflowEffectHarness(holder)
            }
        }

        holder.emit(AppUiEffect.ShowEndSessionConfirmation)
        composeRule.onNodeWithTag("effect-end-session-confirmation").assertIsDisplayed()
        composeRule.onNodeWithTag("effect-route-startup").assertIsDisplayed()

        holder.emit(AppUiEffect.ShowLeaveSessionConfirmation)
        composeRule.onNodeWithTag("effect-leave-session-confirmation").assertIsDisplayed()
        composeRule.onNodeWithTag("effect-route-startup").assertIsDisplayed()
    }

    @Test
    fun transientMessageEffectDoesNotNavigate() {
        val holder = FakeWorkflowEffectHolder()
        composeRule.setContent {
            SilentDiscoTheme {
                WorkflowEffectHarness(holder)
            }
        }

        holder.emit(AppUiEffect.ShowTransientMessage("Invite code copied"))
        composeRule.onNodeWithTag("effect-transient-message").assertIsDisplayed()
        composeRule.onNodeWithTag("effect-route-startup").assertIsDisplayed()
        composeRule.onNodeWithTag("effect-route-home").assertDoesNotExist()
    }
}

private class FakeWorkflowEffectHolder {
    private val channel = Channel<AppUiEffect>(capacity = Channel.BUFFERED)
    val effects = channel.receiveAsFlow()

    fun emit(effect: AppUiEffect) {
        check(channel.trySend(effect).isSuccess) { "Failed to enqueue $effect" }
    }
}

@Composable
private fun WorkflowEffectHarness(
    holder: FakeWorkflowEffectHolder,
    onControllerReady: (NavHostController) -> Unit = {},
) {
    val navController = rememberNavController()
    var showEndConfirmation by remember { mutableStateOf(false) }
    var showLeaveConfirmation by remember { mutableStateOf(false) }
    var transientMessage by remember { mutableStateOf<String?>(null) }

    SideEffect { onControllerReady(navController) }

    LaunchedEffect(holder, navController) {
        holder.effects.collect { effect ->
            when (effect) {
                AppUiEffect.NavigateHome -> withContext(Dispatchers.Main.immediate) {
                    navController.navigateHomeAndClearWorkflow()
                }
                AppUiEffect.NavigateHostDashboard -> withContext(Dispatchers.Main.immediate) {
                    navController.navigateSingleTop(AppRoutes.HostDashboard)
                }
                AppUiEffect.NavigateListenerPlayback -> withContext(Dispatchers.Main.immediate) {
                    navController.navigateSingleTop(AppRoutes.ListenerPlayback)
                }
                AppUiEffect.ShowEndSessionConfirmation -> showEndConfirmation = true
                AppUiEffect.ShowLeaveSessionConfirmation -> showLeaveConfirmation = true
                is AppUiEffect.ShowTransientMessage -> transientMessage = effect.message
            }
        }
    }

    NavHost(
        navController = navController,
        startDestination = AppRoutes.Startup,
    ) {
        composable(AppRoutes.Startup) { EffectRouteMarker("effect-route-startup") }
        composable(AppRoutes.Home) { EffectRouteMarker("effect-route-home") }
        composable(AppRoutes.HostDashboard) { EffectRouteMarker("effect-route-host-dashboard") }
        composable(AppRoutes.ListenerPlayback) { EffectRouteMarker("effect-route-listener-playback") }
    }

    if (showEndConfirmation) {
        Text("End session confirmation", Modifier.testTag("effect-end-session-confirmation"))
    }
    if (showLeaveConfirmation) {
        Text("Leave session confirmation", Modifier.testTag("effect-leave-session-confirmation"))
    }
    transientMessage?.let {
        Text(it, Modifier.testTag("effect-transient-message"))
    }
}

@Composable
private fun EffectRouteMarker(tag: String) {
    Text(tag, Modifier.testTag(tag))
}
