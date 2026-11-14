package com.ekkus.silentdisco.app

import androidx.activity.compose.ManagedActivityResultLauncher
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable

/**
 * Runtime nearby-device permissions: records each result in the view-model,
 * then completes [request] as granted only when every permission its pending
 * action needs is now held.
 */
@Composable
internal fun rememberRuntimePermissionLauncher(
    onPermissionResult: (permission: String, granted: Boolean) -> Unit,
    request: PermissionRequestState,
    uiState: AppUiState,
): ManagedActivityResultLauncher<Array<String>, Map<String, Boolean>> =
    rememberLauncherForActivityResult(
        contract = ActivityResultContracts.RequestMultiplePermissions(),
    ) { results ->
        results.forEach { (permission, granted) ->
            onPermissionResult(permission, granted)
        }
        val granted = request.pendingAction?.context?.requiredPermissions()?.all { permission ->
            results[permission.androidPermission] == true ||
                uiState.permissions.firstOrNull { it.permission == permission }?.granted == true
        } == true
        request.complete(granted)
    }
