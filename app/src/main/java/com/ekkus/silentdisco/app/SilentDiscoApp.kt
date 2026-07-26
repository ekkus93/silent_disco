package com.ekkus.silentdisco.app

import android.content.Intent
import android.net.Uri
import android.provider.Settings
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import com.ekkus.silentdisco.BuildConfig
import com.ekkus.silentdisco.core.model.AppRole
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.feature.diagnostics.ConnectionHelpScreen
import com.ekkus.silentdisco.feature.diagnostics.DiagnosticsScreen
import com.ekkus.silentdisco.feature.home.RoleFirstHomeScreen
import com.ekkus.silentdisco.feature.host.HostAccessSetupScreen
import com.ekkus.silentdisco.feature.host.HostDashboardScreen
import com.ekkus.silentdisco.feature.host.HostMusicSetupScreen
import com.ekkus.silentdisco.feature.listener.ListenerPlaybackV2Screen
import com.ekkus.silentdisco.feature.listener.NearbySessionsScreen
import com.ekkus.silentdisco.feature.listener.SessionJoinScreen
import com.ekkus.silentdisco.feature.settings.SettingsScreen
import com.ekkus.silentdisco.feature.startup.StartupGateScreen
import com.ekkus.silentdisco.ui.components.ConfirmationSheet
import java.time.Instant

@Composable
fun SilentDiscoApp(viewModel: MainViewModel) {
    val uiState by viewModel.uiState.collectAsStateWithLifecycle()
    val navController = rememberNavController()
    val snackbarHostState = remember { SnackbarHostState() }
    val context = LocalContext.current

    var pendingPermissionContext by remember { mutableStateOf<PermissionRequestContext?>(null) }
    var pendingPermissionContinuation by remember { mutableStateOf<(() -> Unit)?>(null) }
    var showEndSessionConfirmation by rememberSaveable { mutableStateOf(false) }
    var showLeaveSessionConfirmation by rememberSaveable { mutableStateOf(false) }
    var showInviteDialog by rememberSaveable { mutableStateOf(false) }
    var approvalInFlightRequestId by rememberSaveable { mutableStateOf<String?>(null) }

    fun shareSupportReport() {
        val report = uiState.buildSupportReport(
            appVersion = BuildConfig.VERSION_NAME,
            generatedAt = Instant.now().toString(),
        )
        val shareIntent = Intent.createChooser(
            Intent(Intent.ACTION_SEND).apply {
                type = "text/plain"
                putExtra(Intent.EXTRA_TEXT, report)
            },
            "Share support report",
        )
        context.startActivity(shareIntent)
    }

    val permissionLauncher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.RequestMultiplePermissions(),
    ) { results ->
        results.forEach { (permission, granted) ->
            viewModel.updatePermission(permission, granted)
        }
        val requestedContext = pendingPermissionContext
        val granted = requestedContext?.requiredPermissions()?.all { permission ->
            results[permission.androidPermission] == true ||
                uiState.permissions.firstOrNull { it.permission == permission }?.granted == true
        } == true
        val continuation = pendingPermissionContinuation
        pendingPermissionContext = null
        pendingPermissionContinuation = null
        if (granted) continuation?.invoke()
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

    fun requestPermissionThen(
        requestContext: PermissionRequestContext,
        continuation: () -> Unit,
    ) {
        if (uiState.hasPermissions(requestContext)) {
            continuation()
        } else {
            pendingPermissionContext = requestContext
            pendingPermissionContinuation = continuation
        }
    }

    LaunchedEffect(uiState.lastMessage) {
        uiState.lastMessage?.takeIf(String::isNotBlank)?.let {
            snackbarHostState.showSnackbar(it)
        }
    }

    LaunchedEffect(uiState.pendingJoinRequests, uiState.approvedListeners) {
        val inFlight = approvalInFlightRequestId ?: return@LaunchedEffect
        if (uiState.pendingJoinRequests.none { it.requestId == inFlight }) {
            approvalInFlightRequestId = null
        }
    }

    Scaffold(
        snackbarHost = { SnackbarHost(hostState = snackbarHostState) },
    ) { paddingValues ->
        NavHost(
            navController = navController,
            startDestination = AppRoutes.Startup,
            modifier = Modifier.padding(paddingValues),
        ) {
            composable(AppRoutes.Startup) {
                var startupNavigationConsumed by rememberSaveable { mutableStateOf(false) }
                LaunchedEffect(uiState.storageState) {
                    if (
                        uiState.storageState == StorageInitializationState.READY &&
                        !startupNavigationConsumed
                    ) {
                        startupNavigationConsumed = true
                        navController.navigate(AppRoutes.Home) {
                            popUpTo(AppRoutes.Startup) { inclusive = true }
                            launchSingleTop = true
                        }
                    }
                }
                StartupGateScreen(
                    uiState = uiState,
                    onRetry = viewModel::retryDomainPersistence,
                    onShareSupportReport = ::shareSupportReport,
                )
            }

            composable(AppRoutes.Home) {
                RoleFirstHomeScreen(
                    uiState = uiState,
                    onHostClick = {
                        viewModel.selectRole(AppRole.HOST)
                        requestPermissionThen(PermissionRequestContext.HOST_NEARBY) {
                            navController.navigateSingleTop(AppRoutes.HostMusicSetup)
                        }
                    },
                    onJoinClick = {
                        viewModel.selectRole(AppRole.LISTENER)
                        requestPermissionThen(PermissionRequestContext.LISTENER_NEARBY) {
                            navController.navigateSingleTop(AppRoutes.NearbySessions)
                        }
                    },
                    onSettingsClick = {
                        navController.navigateSingleTop(AppRoutes.Settings)
                    },
                    onRetryStorage = viewModel::retryDomainPersistence,
                )
            }

            composable(AppRoutes.HostMusicSetup) {
                HostMusicSetupScreen(
                    uiState = uiState,
                    onBack = { navController.popBackStack() },
                    onSessionNameChanged = { viewModel.updateHostForm(sessionName = it) },
                    onChooseAudio = {
                        requestPermissionThen(PermissionRequestContext.AUDIO_FILE) {
                            audioPickerLauncher.launch(arrayOf("audio/*"))
                        }
                    },
                    onNext = {
                        navController.navigateSingleTop(AppRoutes.HostAccessSetup)
                    },
                )
            }

            composable(AppRoutes.HostAccessSetup) {
                HostAccessSetupScreen(
                    uiState = uiState,
                    onBack = { navController.popBackStack() },
                    onApprovalModeChanged = { mode ->
                        viewModel.updateHostForm(approvalMode = mode)
                    },
                    onInviteCodeChanged = { code ->
                        viewModel.updateHostForm(inviteCode = normalizeInviteCode(code))
                    },
                    onGenerateCode = {
                        viewModel.updateHostForm(inviteCode = generateInviteCode())
                    },
                    onStartSession = {
                        if (viewModel.createHostSession()) {
                            navController.navigate(AppRoutes.HostDashboard) {
                                popUpTo(AppRoutes.HostMusicSetup) { inclusive = true }
                                launchSingleTop = true
                            }
                        }
                    },
                )
            }

            composable(AppRoutes.HostDashboard) {
                BackHandler { showEndSessionConfirmation = true }
                HostDashboardScreen(
                    uiState = uiState,
                    onBackRequest = { showEndSessionConfirmation = true },
                    onInvite = { showInviteDialog = true },
                    onApproval = { request, action ->
                        if (approvalInFlightRequestId != null) return@HostDashboardScreen
                        approvalInFlightRequestId = request.requestId
                        when (action) {
                            JoinApprovalAction.REJECT -> viewModel.rejectJoinRequest(request)
                            JoinApprovalAction.APPROVE_ONCE -> {
                                viewModel.updateHostForm(rememberApprovedDevices = false)
                                viewModel.approveJoinRequest(request)
                            }
                            JoinApprovalAction.ALWAYS_ALLOW -> {
                                viewModel.updateHostForm(rememberApprovedDevices = true)
                                viewModel.approveJoinRequest(request)
                            }
                        }
                    },
                    onRemoveListener = viewModel::removeListener,
                    onPlayPause = {
                        if (uiState.hostPlaybackState == PlaybackState.PLAYING) {
                            viewModel.pauseHostPlayback()
                        } else {
                            viewModel.startHostPlayback()
                        }
                    },
                    onStop = viewModel::stopHostPlayback,
                    onEndSessionRequest = { showEndSessionConfirmation = true },
                    onOpenConnectionHelp = {
                        navController.navigateSingleTop(AppRoutes.ConnectionHelp)
                    },
                    onAddDemoJoinRequest = viewModel::addDemoJoinRequest,
                )
            }

            composable(AppRoutes.NearbySessions) {
                val permissionRequired = !uiState.hasPermissions(PermissionRequestContext.LISTENER_NEARBY)
                LaunchedEffect(permissionRequired) {
                    if (!permissionRequired) viewModel.scanForSessions()
                }
                NearbySessionsScreen(
                    uiState = uiState,
                    permissionRequired = permissionRequired,
                    onBack = { navController.popBackStack() },
                    onRequestPermission = {
                        requestPermissionThen(PermissionRequestContext.LISTENER_NEARBY) {
                            viewModel.scanForSessions()
                        }
                    },
                    onRefresh = viewModel::scanForSessions,
                    onSelectSession = { session ->
                        viewModel.selectDiscoveredSession(session)
                        navController.navigateSingleTop(AppRoutes.SessionJoin)
                    },
                )
            }

            composable(AppRoutes.SessionJoin) {
                var playbackNavigationConsumed by rememberSaveable { mutableStateOf(false) }
                LaunchedEffect(uiState.listenerState, uiState.listenerPlaybackState) {
                    if (
                        uiState.listenerState == com.ekkus.silentdisco.core.model.ListenerLifecycleState.PLAYING &&
                        uiState.listenerPlaybackState == PlaybackState.PLAYING &&
                        !playbackNavigationConsumed
                    ) {
                        playbackNavigationConsumed = true
                        navController.navigateSingleTop(AppRoutes.ListenerPlayback)
                    }
                }
                val cancelJoin: () -> Unit = {
                    viewModel.cancelJoin()
                    val returned = navController.popBackStack(AppRoutes.NearbySessions, inclusive = false)
                    if (!returned) navController.navigateSingleTop(AppRoutes.NearbySessions)
                }
                SessionJoinScreen(
                    uiState = uiState,
                    onInviteCodeChanged = viewModel::updateInviteCode,
                    onJoin = viewModel::requestJoin,
                    onCancel = cancelJoin,
                    onRetry = viewModel::retryJoin,
                    onReturnToSessions = cancelJoin,
                )
            }

            composable(AppRoutes.ListenerPlayback) {
                BackHandler { showLeaveSessionConfirmation = true }
                ListenerPlaybackV2Screen(
                    uiState = uiState,
                    onBackRequest = { showLeaveSessionConfirmation = true },
                    onVolumeChanged = viewModel::setLocalVolume,
                    onFixConnection = {
                        navController.navigateSingleTop(AppRoutes.ConnectionHelp)
                    },
                    onLeaveRequest = { showLeaveSessionConfirmation = true },
                )
            }

            composable(AppRoutes.ConnectionHelp) {
                ConnectionHelpScreen(
                    uiState = uiState,
                    onBack = { navController.popBackStack() },
                    onResynchronize = viewModel::manualResync,
                    onReconnect = viewModel::retryJoin,
                    onShareSupportReport = ::shareSupportReport,
                    onAdvancedDiagnostics = {
                        navController.navigateSingleTop(AppRoutes.AdvancedDiagnostics)
                    },
                )
            }

            composable(AppRoutes.AdvancedDiagnostics) {
                DiagnosticsScreen(
                    uiState = uiState,
                    onBack = { navController.popBackStack() },
                    onManualResync = viewModel::manualResync,
                    onAdjustTuning = viewModel::adjustTuning,
                    onShare = { shareSupportReport() },
                )
            }

            composable(AppRoutes.Settings) {
                SettingsScreen(
                    uiState = uiState,
                    trustedDeviceManagementAvailable = false,
                    onBack = { navController.popBackStack() },
                    onOpenSystemSettings = {
                        context.startActivity(
                            Intent(
                                Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                                Uri.parse("package:${context.packageName}"),
                            ),
                        )
                    },
                    onOpenTrustedDevices = {},
                    onOpenAdvancedDiagnostics = {
                        navController.navigateSingleTop(AppRoutes.AdvancedDiagnostics)
                    },
                )
            }
        }
    }

    pendingPermissionContext?.let { requestContext ->
        AlertDialog(
            onDismissRequest = {
                pendingPermissionContext = null
                pendingPermissionContinuation = null
            },
            title = { Text(requestContext.title) },
            text = { Text(requestContext.explanation) },
            dismissButton = {
                TextButton(
                    onClick = {
                        pendingPermissionContext = null
                        pendingPermissionContinuation = null
                    },
                ) {
                    Text("Not now")
                }
            },
            confirmButton = {
                Button(
                    onClick = {
                        permissionLauncher.launch(requestContext.androidPermissions())
                    },
                ) {
                    Text("Continue")
                }
            },
        )
    }

    ConfirmationSheet(
        visible = showEndSessionConfirmation,
        title = "End this session?",
        detail = "Playback will stop for ${uiState.hostDiagnostics.connectedListenerCount} connected listener(s).",
        safeActionLabel = "Keep hosting",
        destructiveActionLabel = "End session",
        onDismiss = { showEndSessionConfirmation = false },
        onConfirm = {
            showEndSessionConfirmation = false
            viewModel.endSession()
            navController.navigateHomeAndClearWorkflow()
        },
        testTag = "end-session-confirmation",
    )

    ConfirmationSheet(
        visible = showLeaveSessionConfirmation,
        title = "Leave this session?",
        detail = "Audio playback on this phone will stop.",
        safeActionLabel = "Stay",
        destructiveActionLabel = "Leave session",
        onDismiss = { showLeaveSessionConfirmation = false },
        onConfirm = {
            showLeaveSessionConfirmation = false
            viewModel.leaveSession()
            navController.navigateHomeAndClearWorkflow()
        },
        testTag = "leave-session-confirmation",
    )

    if (showInviteDialog) {
        AlertDialog(
            onDismissRequest = { showInviteDialog = false },
            title = { Text("Invite listeners") },
            text = {
                Text(
                    buildString {
                        appendLine(uiState.hostForm.sessionName)
                        appendLine("Ask listeners to open Silent Disco and find this nearby session.")
                        if (uiState.hostForm.inviteCode.isNotBlank()) {
                            append("Invite code: ${uiState.hostForm.inviteCode}")
                        }
                    },
                )
            },
            confirmButton = {
                TextButton(onClick = { showInviteDialog = false }) {
                    Text("Done")
                }
            },
        )
    }
}
