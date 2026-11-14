package com.ekkus.silentdisco.app

import androidx.compose.runtime.State
import androidx.navigation.NavHostController
import com.ekkus.silentdisco.core.transport.DiscoveryLifecycleController
import com.ekkus.silentdisco.feature.settings.TrustedDevicesUiState
import com.ekkus.silentdisco.feature.settings.TrustedDevicesViewModel

/**
 * What every feature's navigation graph needs from `SilentDiscoApp`: the
 * view-models, the navigation controller, the collected UI state, and the
 * app-level actions that start other activities or ask for permissions.
 *
 * The state getters read their [State] inside composition, so a destination
 * that reads `uiState` recomposes when it changes, as it did inline.
 */
internal class AppNavContext(
    val viewModel: MainViewModel,
    val workflowViewModel: WorkflowViewModel,
    val trustedDevicesViewModel: TrustedDevicesViewModel,
    val p2ViewModel: P2ViewModel,
    val navController: NavHostController,
    private val uiStateSource: State<AppUiState>,
    private val p2UiStateSource: State<P2UiState>,
    private val trustedDevicesUiStateSource: State<TrustedDevicesUiState>,
    private val captureStatusSource: State<String>,
    val discoveryLifecycleController: DiscoveryLifecycleController,
    val requestPermissionThen: (PermissionAction) -> Unit,
    val shareSupportReport: () -> Unit,
    val openSystemSettings: () -> Unit,
    val launchQrScanner: () -> Unit,
    val showInviteDialog: () -> Unit,
) {
    val uiState: AppUiState get() = uiStateSource.value
    val p2UiState: P2UiState get() = p2UiStateSource.value
    val trustedDevicesUiState: TrustedDevicesUiState get() = trustedDevicesUiStateSource.value
    val captureStatus: String get() = captureStatusSource.value
}
