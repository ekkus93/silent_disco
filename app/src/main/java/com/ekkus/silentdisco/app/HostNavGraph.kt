package com.ekkus.silentdisco.app

import androidx.navigation.NavGraphBuilder
import androidx.activity.compose.BackHandler
import androidx.navigation.compose.composable
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.feature.host.HostAccessSetupScreen
import com.ekkus.silentdisco.feature.host.HostDashboardScreen
import com.ekkus.silentdisco.feature.host.HostMusicSetupScreen
import com.ekkus.silentdisco.feature.host.HostQueueManagementScreen
import com.ekkus.silentdisco.feature.host.HostReadyScreen

/** The host flow: music, access, ready, the live dashboard and its queue. */
internal fun NavGraphBuilder.hostNavGraph(nav: AppNavContext) = with(nav) {
        composable(AppRoutes.HostMusicSetup) {
            HostMusicSetupScreen(
                uiState = uiState,
                onBack = { navController.popBackStack() },
                onSessionNameChanged = { viewModel.updateHostForm(sessionName = it) },
                onChooseAudio = {
                    requestPermissionThen(PermissionAction.PICK_SETUP_AUDIO)
                },
                onRemoveAudio = viewModel::removeSetupAudio,
                onMoveAudio = viewModel::moveSetupAudio,
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
                onNext = { navController.navigateSingleTop(AppRoutes.HostReady) },
            )
        }

        composable(AppRoutes.HostReady) {
            // Hardware/system Back must discard the same saved Access
            // submission lock as the toolbar Back action.
            BackHandler(onBack = navController::returnFromHostReadyToAccess)
            HostReadyScreen(
                uiState = uiState,
                onBack = { navController.returnFromHostReadyToAccess() },
                onStartSession = viewModel::createHostSession,
                onOpenSettings = openSystemSettings,
                onShareSupportReport = shareSupportReport,
            )
        }

        composable(AppRoutes.HostDashboard) {
            BackHandler(onBack = workflowViewModel::requestEndSessionConfirmation)
            HostDashboardScreen(
                uiState = uiState,
                onBackRequest = workflowViewModel::requestEndSessionConfirmation,
                onInvite = showInviteDialog,
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
                onAddSongs = {
                    requestPermissionThen(PermissionAction.PICK_LIVE_AUDIO)
                },
                onManageQueue = { navController.navigateSingleTop(AppRoutes.HostQueueManagement) },
            )
        }

        composable(AppRoutes.HostQueueManagement) {
            HostQueueManagementScreen(
                uiState = uiState,
                onBack = { navController.popBackStack() },
                onAddSongs = {
                    requestPermissionThen(PermissionAction.PICK_LIVE_AUDIO)
                },
                onReorderQueue = viewModel::reorderLiveQueue,
                onRemoveQueueEntry = viewModel::removeLiveQueueEntry,
            )
        }
}
