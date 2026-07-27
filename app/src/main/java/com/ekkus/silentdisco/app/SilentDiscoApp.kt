package com.ekkus.silentdisco.app

import android.Manifest
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
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
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import com.ekkus.silentdisco.BuildConfig
import com.ekkus.silentdisco.core.model.AppRole
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.transport.DiscoveryLifecycleController
import com.ekkus.silentdisco.feature.diagnostics.ConnectionHelpScreen
import com.ekkus.silentdisco.feature.diagnostics.DiagnosticsScreen
import com.ekkus.silentdisco.feature.home.RoleFirstHomeScreen
import com.ekkus.silentdisco.feature.host.HostAccessSetupScreen
import com.ekkus.silentdisco.feature.host.HostDashboardScreen
import com.ekkus.silentdisco.feature.host.HostMusicSetupScreen
import com.ekkus.silentdisco.feature.host.InviteSessionSheet
import com.ekkus.silentdisco.feature.listener.ListenerPlaybackV2Screen
import com.ekkus.silentdisco.feature.listener.NearbySessionsScreen
import com.ekkus.silentdisco.feature.listener.SessionJoinScreen
import com.ekkus.silentdisco.feature.p2.QrHostInvitationDialog
import com.ekkus.silentdisco.feature.p2.TrustedHostsScreen
import com.ekkus.silentdisco.feature.p2.VerifiedQrInvitationDialog
import com.ekkus.silentdisco.feature.settings.SettingsScreen
import com.ekkus.silentdisco.feature.settings.TrustedDevicesScreen
import com.ekkus.silentdisco.feature.settings.TrustedDevicesViewModel
import com.ekkus.silentdisco.feature.startup.StartupGateScreen
import com.ekkus.silentdisco.ui.components.ConfirmationSheet
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import java.time.Instant
import kotlinx.coroutines.flow.collect

@Composable
fun SilentDiscoApp(
    viewModel: MainViewModel,
    workflowViewModel: WorkflowViewModel,
    trustedDevicesViewModel: TrustedDevicesViewModel,
    p2ViewModel: P2ViewModel,
) {
    val uiState by viewModel.uiState.collectAsStateWithLifecycle()
    val trustedDevicesUiState by trustedDevicesViewModel.uiState.collectAsStateWithLifecycle()
    val p2UiState by p2ViewModel.uiState.collectAsStateWithLifecycle()
    val navController = rememberNavController()
    val snackbarHostState = remember { SnackbarHostState() }
    val context = LocalContext.current
    val discoveryLifecycleController = remember(context) { DiscoveryLifecycleController(context) }

    var pendingPermissionContext by remember { mutableStateOf<PermissionRequestContext?>(null) }
    var pendingPermissionContinuation by remember { mutableStateOf<(() -> Unit)?>(null) }
    var showEndSessionConfirmation by rememberSaveable { mutableStateOf(false) }
    var showLeaveSessionConfirmation by rememberSaveable { mutableStateOf(false) }
    var showInviteDialog by rememberSaveable { mutableStateOf(false) }
    var showVerifiedQrDialog by rememberSaveable { mutableStateOf(false) }
    var pendingQrJoin by rememberSaveable { mutableStateOf(false) }
    var qrJoinScanObserved by rememberSaveable { mutableStateOf(false) }
    var showCameraRationale by rememberSaveable { mutableStateOf(false) }
    var showCameraDenied by rememberSaveable { mutableStateOf(false) }

    fun sharePlainText(title: String, text: String) {
        val shareIntent = Intent.createChooser(
            Intent(Intent.ACTION_SEND).apply {
                type = "text/plain"
                putExtra(Intent.EXTRA_TEXT, text)
            },
            title,
        )
        context.startActivity(shareIntent)
    }

    fun shareSupportReport() {
        val report = uiState.buildSupportReport(
            appVersion = BuildConfig.VERSION_NAME,
            generatedAt = Instant.now().toString(),
        )
        sharePlainText("Share support report", report)
    }

    fun openSystemSettings() {
        context.startActivity(
            Intent(
                Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                Uri.parse("package:${context.packageName}"),
            ),
        )
    }

    fun qrScanOptions(): ScanOptions = ScanOptions().apply {
        setDesiredBarcodeFormats(ScanOptions.QR_CODE)
        setPrompt("Scan a signed Silent Disco invitation")
        setBeepEnabled(false)
        setOrientationLocked(false)
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

    val qrScannerLauncher = rememberLauncherForActivityResult(ScanContract()) { result ->
        val payload = result.contents
        if (payload.isNullOrBlank()) {
            workflowViewModel.showTransientMessage("QR scanning cancelled")
        } else {
            p2ViewModel.validateScannedQr(payload)
        }
    }

    val cameraPermissionLauncher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.RequestPermission(),
    ) { granted ->
        if (granted) {
            qrScannerLauncher.launch(qrScanOptions())
        } else {
            showCameraDenied = true
        }
    }

    fun launchQrScanner() {
        if (
            ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
            PackageManager.PERMISSION_GRANTED
        ) {
            qrScannerLauncher.launch(qrScanOptions())
        } else {
            showCameraRationale = true
        }
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

    fun startVerifiedQrJoin() {
        val invitation = p2UiState.validatedInvitation ?: return
        showVerifiedQrDialog = false
        pendingQrJoin = true
        qrJoinScanObserved = false
        viewModel.selectRole(AppRole.LISTENER)
        requestPermissionThen(PermissionRequestContext.LISTENER_NEARBY) {
            navController.navigateSingleTop(AppRoutes.NearbySessions)
            viewModel.scanForSessions()
        }
        workflowViewModel.showTransientMessage("Looking for ${invitation.sessionName} nearby…")
    }

    LaunchedEffect(uiState) {
        workflowViewModel.onUiStateChanged(uiState)
        p2ViewModel.observeAppState(uiState)
    }

    LaunchedEffect(workflowViewModel, navController) {
        workflowViewModel.effects.collect { effect ->
            when (effect) {
                AppUiEffect.NavigateHome -> navController.navigateHomeAndClearWorkflow()
                AppUiEffect.NavigateHostDashboard -> {
                    navController.navigate(AppRoutes.HostDashboard) {
                        popUpTo(AppRoutes.HostMusicSetup) { inclusive = true }
                        launchSingleTop = true
                    }
                }
                AppUiEffect.NavigateListenerPlayback -> navController.navigateSingleTop(AppRoutes.ListenerPlayback)
                AppUiEffect.ShowEndSessionConfirmation -> showEndSessionConfirmation = true
                AppUiEffect.ShowLeaveSessionConfirmation -> showLeaveSessionConfirmation = true
                is AppUiEffect.ShowTransientMessage -> snackbarHostState.showSnackbar(effect.message)
            }
        }
    }

    LaunchedEffect(uiState.lastMessage) {
        uiState.lastMessage?.takeIf(String::isNotBlank)?.let { snackbarHostState.showSnackbar(it) }
    }

    LaunchedEffect(p2UiState.lastMessage) {
        p2UiState.lastMessage?.takeIf(String::isNotBlank)?.let {
            snackbarHostState.showSnackbar(it)
            p2ViewModel.consumeMessage()
        }
    }

    LaunchedEffect(p2UiState.lastError) {
        p2UiState.lastError?.takeIf(String::isNotBlank)?.let {
            snackbarHostState.showSnackbar(it)
            p2ViewModel.consumeError()
        }
    }

    LaunchedEffect(
        p2UiState.validatedInvitation?.sessionId,
        p2UiState.validatedInvitation?.expiresAtMs,
    ) {
        showVerifiedQrDialog = p2UiState.validatedInvitation != null
    }

    LaunchedEffect(
        p2UiState.rejoinTarget?.sessionId,
        p2UiState.recentAvailability,
        uiState.discoveredSessions,
    ) {
        val target = p2UiState.rejoinTarget
        if (target != null && p2UiState.recentAvailability == RecentAvailability.AVAILABLE) {
            val live = uiState.discoveredSessions.firstOrNull { it.id == target.sessionId }
            if (live != null) {
                p2ViewModel.cancelRecentSessionCheck()
                viewModel.selectDiscoveredSession(live)
                navController.navigateSingleTop(AppRoutes.SessionJoin)
            }
        }
    }

    LaunchedEffect(pendingQrJoin, uiState.isScanning, uiState.discoveredSessions) {
        if (!pendingQrJoin) return@LaunchedEffect
        if (uiState.isScanning) {
            qrJoinScanObserved = true
            return@LaunchedEffect
        }
        if (!qrJoinScanObserved) return@LaunchedEffect
        val invitation = p2UiState.validatedInvitation ?: return@LaunchedEffect
        qrJoinScanObserved = false
        val live = uiState.discoveredSessions.firstOrNull { it.id == invitation.sessionId }
        if (live != null) {
            pendingQrJoin = false
            viewModel.selectDiscoveredSession(live)
            viewModel.updateInviteCode(invitation.inviteCode.orEmpty())
            p2ViewModel.dismissValidatedInvitation()
            navController.navigateSingleTop(AppRoutes.SessionJoin)
        } else {
            pendingQrJoin = false
            p2ViewModel.dismissValidatedInvitation()
            snackbarHostState.showSnackbar(
                "The signed invitation is valid, but that exact session is not currently nearby.",
            )
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
                    onSettingsClick = { navController.navigateSingleTop(AppRoutes.Settings) },
                    onRetryStorage = viewModel::retryDomainPersistence,
                    recentSession = p2UiState.mostRecentSession,
                    recentAvailability = p2UiState.recentAvailability,
                    onCheckRecentSession = p2UiState.mostRecentSession?.let { recent ->
                        {
                            p2ViewModel.requestRecentSessionCheck(recent)
                            viewModel.selectRole(AppRole.LISTENER)
                            requestPermissionThen(PermissionRequestContext.LISTENER_NEARBY) {
                                navController.navigateSingleTop(AppRoutes.NearbySessions)
                            }
                        }
                    },
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
                    onNext = { navController.navigateSingleTop(AppRoutes.HostAccessSetup) },
                )
            }

            composable(AppRoutes.HostAccessSetup) {
                HostAccessSetupScreen(
                    uiState = uiState,
                    onBack = { navController.popBackStack() },
                    onApprovalModeChanged = { mode -> viewModel.updateHostForm(approvalMode = mode) },
                    onInviteCodeChanged = { code ->
                        viewModel.updateHostForm(inviteCode = normalizeInviteCode(code))
                    },
                    onGenerateCode = { viewModel.updateHostForm(inviteCode = generateInviteCode()) },
                    onStartSession = {
                        workflowViewModel.onHostSessionCreationResult(viewModel.createHostSession())
                    },
                    onOpenSettings = ::openSystemSettings,
                    onShareSupportReport = ::shareSupportReport,
                )
            }

            composable(AppRoutes.HostDashboard) {
                BackHandler(onBack = workflowViewModel::requestEndSessionConfirmation)
                HostDashboardScreen(
                    uiState = uiState,
                    onBackRequest = workflowViewModel::requestEndSessionConfirmation,
                    onInvite = { showInviteDialog = true },
                    onApproval = { request, action ->
                        workflowViewModel.handleJoinApproval(viewModel, request, action)
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
                    onEndSessionRequest = workflowViewModel::requestEndSessionConfirmation,
                    onOpenConnectionHelp = {
                        navController.navigateSingleTop(AppRoutes.ConnectionHelp)
                    },
                    onAddDemoJoinRequest = viewModel::addDemoJoinRequest,
                )
            }

            composable(AppRoutes.NearbySessions) {
                DisposableEffect(discoveryLifecycleController) {
                    onDispose {
                        discoveryLifecycleController.stopDiscovery(workflowViewModel::showTransientMessage)
                    }
                }
                val permissionRequired = !uiState.hasPermissions(PermissionRequestContext.LISTENER_NEARBY)
                LaunchedEffect(permissionRequired) {
                    if (!permissionRequired) viewModel.scanForSessions()
                }
                NearbySessionsScreen(
                    uiState = uiState,
                    permissionRequired = permissionRequired,
                    onBack = {
                        p2ViewModel.cancelRecentSessionCheck()
                        navController.popBackStack()
                    },
                    onRequestPermission = {
                        requestPermissionThen(PermissionRequestContext.LISTENER_NEARBY) {}
                    },
                    onRefresh = viewModel::scanForSessions,
                    onSelectSession = { session ->
                        p2ViewModel.cancelRecentSessionCheck()
                        viewModel.selectDiscoveredSession(session)
                        navController.navigateSingleTop(AppRoutes.SessionJoin)
                    },
                    onScanQr = ::launchQrScanner,
                    trustedVerifiedSessionIds = p2UiState.trustedVerifiedSessionIds(),
                )
            }

            composable(AppRoutes.SessionJoin) {
                val cancelJoin: () -> Unit = {
                    discoveryLifecycleController.cancelJoinTransport(workflowViewModel::showTransientMessage)
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
                    onOpenSettings = ::openSystemSettings,
                    onShareSupportReport = ::shareSupportReport,
                )
            }

            composable(AppRoutes.ListenerPlayback) {
                BackHandler(onBack = workflowViewModel::requestLeaveSessionConfirmation)
                ListenerPlaybackV2Screen(
                    uiState = uiState,
                    onBackRequest = workflowViewModel::requestLeaveSessionConfirmation,
                    onVolumeChanged = viewModel::setLocalVolume,
                    onFixConnection = {
                        navController.navigateSingleTop(AppRoutes.ConnectionHelp)
                    },
                    onLeaveRequest = workflowViewModel::requestLeaveSessionConfirmation,
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
                    trustedDeviceManagementAvailable = true,
                    onBack = { navController.popBackStack() },
                    onOpenSystemSettings = ::openSystemSettings,
                    onOpenTrustedDevices = {
                        navController.navigateSingleTop(AppRoutes.TrustedDevices)
                    },
                    onOpenAdvancedDiagnostics = {
                        navController.navigateSingleTop(AppRoutes.AdvancedDiagnostics)
                    },
                    onOpenTrustedHosts = {
                        navController.navigateSingleTop(AppRoutes.TrustedHosts)
                    },
                )
            }

            composable(AppRoutes.TrustedDevices) {
                LaunchedEffect(Unit) { trustedDevicesViewModel.refresh() }
                TrustedDevicesScreen(
                    uiState = trustedDevicesUiState,
                    onBack = { navController.popBackStack() },
                    onRefresh = trustedDevicesViewModel::refresh,
                    onDelete = trustedDevicesViewModel::deleteDevice,
                )
            }

            composable(AppRoutes.TrustedHosts) {
                TrustedHostsScreen(
                    uiState = p2UiState,
                    onBack = { navController.popBackStack() },
                    onDelete = p2ViewModel::deleteTrustedHost,
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

    if (showCameraRationale) {
        AlertDialog(
            onDismissRequest = { showCameraRationale = false },
            title = { Text("Use the camera to scan a signed invitation") },
            text = {
                Text(
                    "Silent Disco uses the camera only while scanning a QR code. The image is decoded on this phone and is not saved.",
                )
            },
            dismissButton = {
                TextButton(onClick = { showCameraRationale = false }) { Text("Not now") }
            },
            confirmButton = {
                Button(
                    onClick = {
                        showCameraRationale = false
                        cameraPermissionLauncher.launch(Manifest.permission.CAMERA)
                    },
                ) {
                    Text("Continue")
                }
            },
        )
    }

    if (showCameraDenied) {
        AlertDialog(
            onDismissRequest = { showCameraDenied = false },
            title = { Text("Camera access is required") },
            text = {
                Text("Allow camera access in system settings to scan signed QR invitations.")
            },
            dismissButton = {
                TextButton(onClick = { showCameraDenied = false }) { Text("Cancel") }
            },
            confirmButton = {
                Button(
                    onClick = {
                        showCameraDenied = false
                        openSystemSettings()
                    },
                ) {
                    Text("Open Settings")
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
            workflowViewModel.navigateHome()
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
            discoveryLifecycleController.cancelJoinTransport(workflowViewModel::showTransientMessage)
            viewModel.leaveSession()
            workflowViewModel.navigateHome()
        },
        testTag = "leave-session-confirmation",
    )

    if (showInviteDialog) {
        InviteSessionSheet(
            uiState = uiState,
            onDismiss = { showInviteDialog = false },
            onCopyCode = { code ->
                val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                clipboard.setPrimaryClip(ClipData.newPlainText("Silent Disco invite code", code))
                workflowViewModel.showTransientMessage("Invite code copied")
            },
            onShareInstructions = { instructions ->
                sharePlainText("Share Silent Disco invitation", instructions)
            },
            onShowSignedQr = {
                showInviteDialog = false
                p2ViewModel.prepareHostQr(uiState)
            },
        )
    }

    p2UiState.hostQrPayload?.let { payload ->
        QrHostInvitationDialog(
            payload = payload,
            onDismiss = p2ViewModel::clearHostQr,
        )
    }

    if (showVerifiedQrDialog) {
        p2UiState.validatedInvitation?.let { invitation ->
            VerifiedQrInvitationDialog(
                invitation = invitation,
                alreadyTrusted = p2UiState.validatedHostIsTrusted(),
                onDismiss = {
                    showVerifiedQrDialog = false
                    p2ViewModel.dismissValidatedInvitation()
                },
                onJoinOnce = ::startVerifiedQrJoin,
                onTrustAndJoin = {
                    if (p2UiState.validatedHostIsTrusted()) {
                        startVerifiedQrJoin()
                    } else {
                        p2ViewModel.trustValidatedHost()
                    }
                },
            )
        }
    }
}
