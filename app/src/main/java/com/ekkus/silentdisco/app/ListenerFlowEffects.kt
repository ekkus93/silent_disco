package com.ekkus.silentdisco.app

import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.navigation.NavHostController

/**
 * The listener's app-level flow effects (God-file refactor TODO §6.5), moved
 * from `SilentDiscoApp` with their keys unchanged and called at the same
 * point, so they launch in the same order:
 * - open the verified-invitation dialog when a scanned QR validates;
 * - rejoin a recent session once it is seen nearby again;
 * - finish a QR join once its discovery scan has run, or say the session is
 *   not nearby.
 */
@Composable
internal fun ListenerFlowEffects(
    uiState: AppUiState,
    p2UiState: P2UiState,
    qrJoin: QrJoinFlowState,
    viewModel: MainViewModel,
    p2ViewModel: P2ViewModel,
    navController: NavHostController,
    snackbarHostState: SnackbarHostState,
) {
    LaunchedEffect(
        p2UiState.validatedInvitation?.sessionId,
        p2UiState.validatedInvitation?.expiresAtMs,
    ) {
        qrJoin.showVerifiedQrDialog = p2UiState.validatedInvitation != null
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

    LaunchedEffect(qrJoin.pendingQrJoin, uiState.isScanning, uiState.discoveredSessions) {
        if (!qrJoin.pendingQrJoin) return@LaunchedEffect
        if (uiState.isScanning) {
            qrJoin.qrJoinScanObserved = true
            return@LaunchedEffect
        }
        if (!qrJoin.qrJoinScanObserved) return@LaunchedEffect
        val invitation = p2UiState.validatedInvitation ?: return@LaunchedEffect
        qrJoin.qrJoinScanObserved = false
        val live = uiState.discoveredSessions.firstOrNull { it.id == invitation.sessionId }
        if (live != null) {
            qrJoin.pendingQrJoin = false
            viewModel.selectDiscoveredSession(live)
            viewModel.updateInviteCode(invitation.inviteCode.orEmpty())
            p2ViewModel.dismissValidatedInvitation()
            navController.navigateSingleTop(AppRoutes.SessionJoin)
        } else {
            qrJoin.pendingQrJoin = false
            p2ViewModel.dismissValidatedInvitation()
            snackbarHostState.showSnackbar(
                "The signed invitation is valid, but that exact session is not currently nearby.",
            )
        }
    }
}
