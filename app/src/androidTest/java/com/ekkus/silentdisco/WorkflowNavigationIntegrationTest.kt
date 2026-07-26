package com.ekkus.silentdisco

import android.content.Context
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
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
import com.ekkus.silentdisco.app.navigateHomeAndClearWorkflow
import com.ekkus.silentdisco.app.navigateSingleTop
import com.google.common.truth.Truth.assertThat
import org.junit.Before
import org.junit.Rule
import org.junit.Test

class WorkflowNavigationIntegrationTest {
    @get:Rule
    val composeRule = createComposeRule()

    private lateinit var navController: TestNavHostController

    @Before
    fun setUpNavigationGraph() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        navController = TestNavHostController(context).apply {
            navigatorProvider.addNavigator(ComposeNavigator())
        }

        composeRule.setContent {
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
    fun singleTopNavigationDoesNotDuplicateCurrentDestination() {
        navigateSingleTop(AppRoutes.Home)
        navigateSingleTop(AppRoutes.Home)

        assertCurrentRoute(AppRoutes.Home)
        val popped = composeRule.runOnIdle { navController.popBackStack() }

        assertThat(popped).isTrue()
        assertCurrentRoute(AppRoutes.Startup)
    }

    @Test
    fun returningHomeClearsEntireHostWorkflow() {
        navigateSingleTop(AppRoutes.Home)
        navigateSingleTop(AppRoutes.HostMusicSetup)
        navigateSingleTop(AppRoutes.HostAccessSetup)
        navigateSingleTop(AppRoutes.HostDashboard)

        composeRule.runOnIdle { navController.navigateHomeAndClearWorkflow() }

        assertCurrentRoute(AppRoutes.Home)
        val popped = composeRule.runOnIdle { navController.popBackStack() }
        assertThat(popped).isFalse()
        assertCurrentRoute(AppRoutes.Home)
    }

    @Test
    fun listenerRecoveryReturnsToPlaybackWithoutLosingWorkflow() {
        navigateSingleTop(AppRoutes.Home)
        navigateSingleTop(AppRoutes.NearbySessions)
        navigateSingleTop(AppRoutes.SessionJoin)
        navigateSingleTop(AppRoutes.ListenerPlayback)
        navigateSingleTop(AppRoutes.ConnectionHelp)

        assertCurrentRoute(AppRoutes.ConnectionHelp)
        val popped = composeRule.runOnIdle { navController.popBackStack() }

        assertThat(popped).isTrue()
        assertCurrentRoute(AppRoutes.ListenerPlayback)
    }

    private fun navigateSingleTop(route: String) {
        composeRule.runOnIdle { navController.navigateSingleTop(route) }
        assertCurrentRoute(route)
    }

    private fun assertCurrentRoute(route: String) {
        val currentRoute = composeRule.runOnIdle {
            navController.currentBackStackEntry?.destination?.route
        }
        assertThat(currentRoute).isEqualTo(route)
        composeRule.onNodeWithTag(routeTag(route)).assertIsDisplayed()
    }

    private fun routeTag(route: String): String = "workflow-route-$route"

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
