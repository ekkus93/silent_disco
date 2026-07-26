package com.ekkus.silentdisco

import android.content.Context
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.navigation.compose.ComposeNavigator
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.testing.TestNavHostController
import androidx.test.core.app.ApplicationProvider
import com.ekkus.silentdisco.app.AppRoutes
import com.ekkus.silentdisco.app.AppUiEffect
import com.ekkus.silentdisco.app.navigateHomeAndClearWorkflow
import com.ekkus.silentdisco.app.navigateSingleTop
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.flow.receiveAsFlow
import org.junit.Before
import org.junit.Rule
import org.junit.Test

class WorkflowEffectIntegrationTest {
    @get:Rule
    val composeRule = createComposeRule()

    private lateinit var navController: TestNavHostController
    private lateinit var workflow: FakeWorkflowStateHolder

    @Before
    fun setUpWorkflow() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        navController = TestNavHostController(context).apply {
            navigatorProvider.addNavigator(ComposeNavigator())
        }
        workflow = FakeWorkflowStateHolder()

        composeRule.setContent {
            LaunchedEffect(workflow) {
                workflow.effects.collect { effect ->
                    when (effect) {
                        AppUiEffect.NavigateHome -> navController.navigateHomeAndClearWorkflow()
                        AppUiEffect.NavigateHostDashboard -> {
                            navController.navigateSingleTop(AppRoutes.HostDashboard)
                        }
                        AppUiEffect.NavigateListenerPlayback -> {
                            navController.navigateSingleTop(AppRoutes.ListenerPlayback)
                        }
                        AppUiEffect.ShowEndSessionConfirmation,
                        AppUiEffect.ShowLeaveSessionConfirmation,
                        is AppUiEffect.ShowTransientMessage -> Unit
                    }
                }
            }

            NavHost(
                navController = navController,
                startDestination = AppRoutes.Startup,
            ) {
                workflowRoutes.forEach { route ->
                    composable(route) {
                        Box(
                            modifier = Modifier
                                .fillMaxSize()
                                .testTag(routeTag(route)),
                        )
                    }
                }
            }
        }
        composeRule.waitForIdle()
    }

    @Test
    fun startupReadyEffectNavigatesHomeAndRemovesStartup() {
        workflow.emit(AppUiEffect.NavigateHome)

        assertCurrentRoute(AppRoutes.Home)
        val popped = composeRule.runOnIdle { navController.popBackStack() }
        assertThat(popped).isFalse()
    }

    @Test
    fun hostCreationEffectNavigatesDashboardAndReturnHomeClearsWorkflow() {
        workflow.emit(AppUiEffect.NavigateHome)
        navigateDirectly(AppRoutes.HostMusicSetup)
        navigateDirectly(AppRoutes.HostAccessSetup)

        workflow.emit(AppUiEffect.NavigateHostDashboard)
        assertCurrentRoute(AppRoutes.HostDashboard)

        workflow.emit(AppUiEffect.NavigateHome)
        assertCurrentRoute(AppRoutes.Home)
        val popped = composeRule.runOnIdle { navController.popBackStack() }
        assertThat(popped).isFalse()
    }

    @Test
    fun repeatedPlaybackEffectsRemainSingleTop() {
        workflow.emit(AppUiEffect.NavigateHome)
        navigateDirectly(AppRoutes.NearbySessions)
        navigateDirectly(AppRoutes.SessionJoin)

        workflow.emit(AppUiEffect.NavigateListenerPlayback)
        workflow.emit(AppUiEffect.NavigateListenerPlayback)
        assertCurrentRoute(AppRoutes.ListenerPlayback)

        val popped = composeRule.runOnIdle { navController.popBackStack() }
        assertThat(popped).isTrue()
        assertCurrentRoute(AppRoutes.SessionJoin)
    }

    private fun navigateDirectly(route: String) {
        composeRule.runOnIdle { navController.navigateSingleTop(route) }
        assertCurrentRoute(route)
    }

    private fun assertCurrentRoute(route: String) {
        composeRule.waitUntil(timeoutMillis = 5_000L) {
            navController.currentBackStackEntry?.destination?.route == route
        }
        composeRule.onNodeWithTag(routeTag(route)).assertIsDisplayed()
    }

    private fun routeTag(route: String): String = "effect-route-$route"

    private class FakeWorkflowStateHolder {
        private val channel = Channel<AppUiEffect>(capacity = Channel.BUFFERED)
        val effects: Flow<AppUiEffect> = channel.receiveAsFlow()

        fun emit(effect: AppUiEffect) {
            check(channel.trySend(effect).isSuccess) { "Failed to emit workflow effect $effect" }
        }
    }

    private companion object {
        val workflowRoutes = listOf(
            AppRoutes.Startup,
            AppRoutes.Home,
            AppRoutes.HostMusicSetup,
            AppRoutes.HostAccessSetup,
            AppRoutes.HostDashboard,
            AppRoutes.NearbySessions,
            AppRoutes.SessionJoin,
            AppRoutes.ListenerPlayback,
            AppRoutes.ConnectionHelp,
            AppRoutes.AdvancedDiagnostics,
            AppRoutes.Settings,
        )
    }
}
