package com.ekkus.silentdisco.app

import androidx.navigation.NavGraphBuilder
import androidx.activity.compose.BackHandler
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.navigation.compose.composable
import com.ekkus.silentdisco.feature.diagnostics.ConnectionHelpScreen
import com.ekkus.silentdisco.feature.listener.ListenerPlaybackV2Screen
import com.ekkus.silentdisco.feature.listener.ManualEndpointScreen
import com.ekkus.silentdisco.feature.listener.NearbySessionsScreen
import com.ekkus.silentdisco.feature.listener.SessionJoinScreen

/** The listener flow: nearby sessions, manual connect, joining, playback and connection help. */
internal fun NavGraphBuilder.listenerNavGraph(nav: AppNavContext) = with(nav) {
        composable(AppRoutes.NearbySessions) {
            DisposableEffect(discoveryLifecycleController) {
                onDispose {
                    discoveryLifecycleController.stopDiscovery(workflowViewModel::showTransientMessage)
                }
            }
            val permissionRequired = !uiState.hasPermissions(PermissionRequestContext.LISTENER_NEARBY)
            LaunchedEffect(permissionRequired, uiState.isScanning) {
                if (!permissionRequired && !uiState.isScanning) viewModel.scanForSessions()
            }
            NearbySessionsScreen(
                uiState = uiState,
                permissionRequired = permissionRequired,
                onBack = {
                    p2ViewModel.cancelRecentSessionCheck()
                    navController.popBackStack()
                },
                onRequestPermission = {
                    requestPermissionThen(PermissionAction.LISTENER_PERMISSION_ONLY)
                },
                onRefresh = viewModel::scanForSessions,
                onSelectSession = { session ->
                    p2ViewModel.cancelRecentSessionCheck()
                    viewModel.selectDiscoveredSession(session)
                    navController.navigateSingleTop(AppRoutes.SessionJoin)
                },
                onScanQr = launchQrScanner,
                onConnectManually = {
                    navController.navigateSingleTop(AppRoutes.ManualConnect)
                },
                trustedVerifiedSessionIds = p2UiState.trustedVerifiedSessionIds(),
            )
        }

        composable(AppRoutes.ManualConnect) {
            val cancelManualConnect: () -> Unit = {
                viewModel.cancelManualEndpointConnect()
                navController.popBackStack()
            }
            ManualEndpointScreen(
                uiState = uiState,
                onBack = cancelManualConnect,
                onInputChanged = viewModel::updateManualEndpointInput,
                onInviteCodeChanged = viewModel::updateManualEndpointInviteCode,
                onConnect = viewModel::connectManualEndpoint,
                onCancel = cancelManualConnect,
            )
        }

        composable(AppRoutes.SessionJoin) {
            val cancelJoin: () -> Unit = {
                if (viewModel.listenerHasPendingWifiDirectJoin) {
                    discoveryLifecycleController.cancelJoinTransport(
                        workflowViewModel::showTransientMessage,
                    )
                }
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
                onOpenSettings = openSystemSettings,
                onShareSupportReport = shareSupportReport,
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
                onReconnect = {
                    viewModel.retryJoin()
                    navController.navigateSingleTop(AppRoutes.NearbySessions)
                },
                onShareSupportReport = shareSupportReport,
                onAdvancedDiagnostics = {
                    navController.navigateSingleTop(AppRoutes.AdvancedDiagnostics)
                },
            )
        }
}
