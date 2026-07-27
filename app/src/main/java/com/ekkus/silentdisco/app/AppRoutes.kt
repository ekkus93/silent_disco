package com.ekkus.silentdisco.app

import androidx.navigation.NavGraph.Companion.findStartDestination
import androidx.navigation.NavHostController

object AppRoutes {
    const val Startup = "startup"
    const val Home = "home"
    const val HostMusicSetup = "host_music_setup"
    const val HostAccessSetup = "host_access_setup"
    const val HostDashboard = "host_dashboard"
    const val NearbySessions = "nearby_sessions"
    const val SessionJoin = "session_join"
    const val ListenerPlayback = "listener_playback"
    const val ConnectionHelp = "connection_help"
    const val AdvancedDiagnostics = "advanced_diagnostics"
    const val Settings = "settings"
    const val TrustedDevices = "trusted_devices"
    const val TrustedHosts = "trusted_hosts"
}

fun NavHostController.navigateSingleTop(route: String) {
    navigate(route) {
        launchSingleTop = true
    }
}

fun NavHostController.navigateHomeAndClearWorkflow() {
    navigate(AppRoutes.Home) {
        popUpTo(graph.findStartDestination().id) {
            inclusive = true
        }
        launchSingleTop = true
    }
}
