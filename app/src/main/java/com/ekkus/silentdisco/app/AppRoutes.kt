package com.ekkus.silentdisco.app

import androidx.navigation.NavGraph.Companion.findStartDestination
import androidx.navigation.NavHostController

object AppRoutes {
    const val Startup = "startup"
    const val Home = "home"
    const val HostMusicSetup = "host_music_setup"
    const val HostAccessSetup = "host_access_setup"
    const val HostReady = "host_ready"
    const val HostDashboard = "host_dashboard"
    const val HostQueueManagement = "host_queue_management"
    const val NearbySessions = "nearby_sessions"
    const val SessionJoin = "session_join"
    const val ManualConnect = "manual_connect"
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

/**
 * Ready -> Access starts a fresh Access destination. Reusing the prior
 * back-stack entry preserves its saveable Next-submission lock and strands
 * the user on a disabled Review button after they press Back.
 */
fun NavHostController.returnFromHostReadyToAccess() {
    navigate(AppRoutes.HostAccessSetup) {
        popUpTo(AppRoutes.HostAccessSetup) { inclusive = true }
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
