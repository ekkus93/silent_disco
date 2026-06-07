package com.ekkus.silentdisco.app

import android.Manifest
import android.content.Intent
import android.net.Uri
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.compose.ui.platform.LocalContext
import androidx.navigation.NavGraph.Companion.findStartDestination
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import com.ekkus.silentdisco.core.model.AppRole
import com.ekkus.silentdisco.feature.diagnostics.DiagnosticsScreen
import com.ekkus.silentdisco.feature.home.HomeScreen
import com.ekkus.silentdisco.feature.host.HostControlScreen
import com.ekkus.silentdisco.feature.host.HostSetupScreen
import com.ekkus.silentdisco.feature.listener.DiscoverSessionsScreen
import com.ekkus.silentdisco.feature.listener.JoinProgressScreen
import com.ekkus.silentdisco.feature.listener.ListenerPlaybackScreen

private object Routes {
    const val Home = "home"
    const val HostSetup = "host_setup"
    const val HostControl = "host_control"
    const val Discover = "discover"
    const val JoinProgress = "join_progress"
    const val ListenerPlayback = "listener_playback"
    const val Diagnostics = "diagnostics"
}

@Composable
fun SilentDiscoApp(viewModel: MainViewModel) {
    val uiState by viewModel.uiState.collectAsStateWithLifecycle()
    val navController = rememberNavController()
    val snackbarHostState = remember { SnackbarHostState() }

    val permissionLauncher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.RequestMultiplePermissions(),
    ) { results ->
        results.forEach { (permission, granted) ->
            viewModel.updatePermission(permission, granted)
        }
    }

    val audioPickerLauncher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.OpenDocument(),
    ) { uri: Uri? ->
        uri ?: return@rememberLauncherForActivityResult
        viewModel.selectAudioFile(
            uri = uri,
            displayName = uri.lastPathSegment ?: "audio-file",
            mimeType = null,
        )
    }

    LaunchedEffect(uiState.lastError, uiState.lastMessage) {
        val message = uiState.lastError ?: uiState.lastMessage
        if (!message.isNullOrBlank()) {
            snackbarHostState.showSnackbar(message)
        }
    }

    Scaffold(
        snackbarHost = { SnackbarHost(hostState = snackbarHostState) },
    ) { paddingValues ->
        NavHost(
            navController = navController,
            startDestination = Routes.Home,
            modifier = Modifier.padding(paddingValues),
        ) {
            composable(Routes.Home) {
                HomeScreen(
                    uiState = uiState,
                    onRequestPermissions = {
                        permissionLauncher.launch(requiredPermissions())
                    },
                    onHostClick = {
                        viewModel.selectRole(AppRole.HOST)
                        navController.navigate(Routes.HostSetup)
                    },
                    onJoinClick = {
                        viewModel.selectRole(AppRole.LISTENER)
                        navController.navigate(Routes.Discover)
                    },
                )
            }
            composable(Routes.HostSetup) {
                HostSetupScreen(
                    uiState = uiState,
                    onBack = { navController.popBackStack() },
                    onSessionNameChanged = { viewModel.updateHostForm(sessionName = it) },
                    onApprovalModeChanged = { viewModel.updateHostForm(approvalMode = it) },
                    onInviteCodeChanged = { viewModel.updateHostForm(inviteCode = it) },
                    onRememberApprovedChanged = { viewModel.updateHostForm(rememberApprovedDevices = it) },
                    onPickAudio = {
                        audioPickerLauncher.launch(arrayOf("audio/*"))
                    },
                    onStartHosting = {
                        if (viewModel.createHostSession()) {
                            navController.navigate(Routes.HostControl)
                        }
                    },
                )
            }
            composable(Routes.HostControl) {
                HostControlScreen(
                    uiState = uiState,
                    onBack = { navController.popBackStack() },
                    onAddDemoJoinRequest = viewModel::addDemoJoinRequest,
                    onApprove = viewModel::approveJoinRequest,
                    onReject = viewModel::rejectJoinRequest,
                    onTrust = viewModel::trustListener,
                    onRemove = viewModel::removeListener,
                    onStart = viewModel::startHostPlayback,
                    onPause = viewModel::pauseHostPlayback,
                    onStop = viewModel::stopHostPlayback,
                    onEndSession = {
                        viewModel.endSession()
                        navController.navigate(Routes.Home) {
                            popUpTo(navController.graph.findStartDestination().id) {
                                inclusive = true
                            }
                        }
                    },
                    onOpenDiagnostics = { navController.navigate(Routes.Diagnostics) },
                )
            }
            composable(Routes.Discover) {
                DiscoverSessionsScreen(
                    uiState = uiState,
                    onBack = { navController.popBackStack() },
                    onRefresh = viewModel::scanForSessions,
                    onSelectSession = {
                        viewModel.selectDiscoveredSession(it)
                        navController.navigate(Routes.JoinProgress)
                    },
                )
            }
            composable(Routes.JoinProgress) {
                val cancelJoin: () -> Unit = {
                    viewModel.cancelJoin()
                    navController.popBackStack(Routes.Discover, inclusive = false)
                }
                JoinProgressScreen(
                    uiState = uiState,
                    onInviteCodeChanged = viewModel::updateInviteCode,
                    onJoin = viewModel::requestJoin,
                    onCancel = cancelJoin,
                    onRetry = viewModel::retryJoin,
                    onContinueWhenPlaying = {
                        navController.navigate(Routes.ListenerPlayback)
                    },
                )
            }
            composable(Routes.ListenerPlayback) {
                ListenerPlaybackScreen(
                    uiState = uiState,
                    onBack = { navController.popBackStack() },
                    onVolumeChanged = viewModel::setLocalVolume,
                    onLeaveSession = {
                        viewModel.leaveSession()
                        navController.navigate(Routes.Home) {
                            popUpTo(navController.graph.findStartDestination().id) {
                                inclusive = true
                            }
                        }
                    },
                    onReconnect = viewModel::retryJoin,
                    onOpenDiagnostics = { navController.navigate(Routes.Diagnostics) },
                )
            }
            composable(Routes.Diagnostics) {
                val context = LocalContext.current
                DiagnosticsScreen(
                    uiState = uiState,
                    onBack = { navController.popBackStack() },
                    onManualResync = viewModel::manualResync,
                    onAdjustTuning = viewModel::adjustTuning,
                    onShare = { text ->
                        val shareIntent = Intent.createChooser(
                            Intent(Intent.ACTION_SEND).apply {
                                type = "text/plain"
                                putExtra(Intent.EXTRA_TEXT, text)
                            },
                            "Share diagnostics",
                        )
                        context.startActivity(shareIntent)
                    },
                )
            }
        }
    }
}

private fun requiredPermissions(): Array<String> = buildList {
    val bluetoothScan = "android.permission.BLUETOOTH_SCAN"
    val bluetoothConnect = "android.permission.BLUETOOTH_CONNECT"
    val bluetoothAdvertise = "android.permission.BLUETOOTH_ADVERTISE"
    val nearbyWifiDevices = "android.permission.NEARBY_WIFI_DEVICES"
    add(Manifest.permission.ACCESS_FINE_LOCATION)
    add(Manifest.permission.ACCESS_COARSE_LOCATION)
    add(Manifest.permission.ACCESS_WIFI_STATE)
    add(Manifest.permission.CHANGE_WIFI_STATE)
    add(bluetoothScan)
    add(bluetoothConnect)
    add(bluetoothAdvertise)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        add(Manifest.permission.READ_MEDIA_AUDIO)
    } else {
        add(Manifest.permission.READ_EXTERNAL_STORAGE)
    }
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        add(nearbyWifiDevices)
    }
}.toTypedArray()
