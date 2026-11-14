package com.ekkus.silentdisco.app

import androidx.navigation.NavGraphBuilder
import android.provider.Settings
import androidx.compose.runtime.LaunchedEffect
import androidx.navigation.compose.composable
import com.ekkus.silentdisco.feature.diagnostics.DiagnosticsScreen
import com.ekkus.silentdisco.feature.p2.TrustedHostsScreen
import com.ekkus.silentdisco.feature.settings.SettingsScreen
import com.ekkus.silentdisco.feature.settings.TrustedDevicesScreen

/** Settings, approved devices, trusted hosts and advanced diagnostics. */
internal fun NavGraphBuilder.settingsNavGraph(nav: AppNavContext) = with(nav) {
        composable(AppRoutes.Settings) {
            SettingsScreen(
                uiState = uiState,
                trustedDeviceManagementAvailable = true,
                onBack = { navController.popBackStack() },
                onOpenSystemSettings = openSystemSettings,
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

        composable(AppRoutes.AdvancedDiagnostics) {
            DiagnosticsScreen(
                uiState = uiState,
                onBack = { navController.popBackStack() },
                onManualResync = viewModel::manualResync,
                onAdjustTuning = viewModel::adjustTuning,
                onShare = { shareSupportReport() },
                callbackCaptureStatus = captureStatus,
                onStartCallbackCapture = viewModel::startCallbackPcmCapture,
                onStopCallbackCapture = viewModel::stopCallbackPcmCapture,
            )
        }
}
