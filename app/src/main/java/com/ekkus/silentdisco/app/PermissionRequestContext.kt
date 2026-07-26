package com.ekkus.silentdisco.app

import android.os.Build
import com.ekkus.silentdisco.core.permissions.AppPermission
import com.ekkus.silentdisco.core.permissions.PermissionCatalogue

enum class PermissionRequestContext(
    val title: String,
    val explanation: String,
) {
    HOST_NEARBY(
        title = "Allow nearby device access",
        explanation = "This lets other phones find and connect to your Silent Disco session.",
    ),
    LISTENER_NEARBY(
        title = "Allow nearby device access",
        explanation = "This lets your phone find Silent Disco sessions near you.",
    ),
    AUDIO_FILE(
        title = "Choose an audio file",
        explanation = "Android’s file picker will grant Silent Disco access only to the file you choose.",
    ),
}

fun PermissionRequestContext.requiredPermissions(
    sdkInt: Int = Build.VERSION.SDK_INT,
): List<AppPermission> = when (this) {
    PermissionRequestContext.HOST_NEARBY -> (
        PermissionCatalogue.wifiDirectPermissions(sdkInt) +
            PermissionCatalogue.bluetoothPermissions(sdkInt)
        ).distinct()

    PermissionRequestContext.LISTENER_NEARBY -> (
        PermissionCatalogue.wifiDirectPermissions(sdkInt) +
            PermissionCatalogue.bluetoothPermissions(sdkInt).filter {
                it == AppPermission.BluetoothScan || it == AppPermission.BluetoothConnect
            }
        ).distinct()

    // ACTION_OPEN_DOCUMENT grants scoped access to the selected URI. Requesting
    // READ_MEDIA_AUDIO here would expose the broader media library unnecessarily.
    PermissionRequestContext.AUDIO_FILE -> emptyList()
}

fun PermissionRequestContext.androidPermissions(
    sdkInt: Int = Build.VERSION.SDK_INT,
): Array<String> = requiredPermissions(sdkInt)
    .map(AppPermission::androidPermission)
    .distinct()
    .toTypedArray()

fun AppUiState.missingPermissions(
    context: PermissionRequestContext,
    sdkInt: Int = Build.VERSION.SDK_INT,
): List<AppPermission> {
    val grantedByPermission = permissions.associate { it.permission to it.granted }
    return context.requiredPermissions(sdkInt).filter { grantedByPermission[it] != true }
}

fun AppUiState.hasPermissions(
    context: PermissionRequestContext,
    sdkInt: Int = Build.VERSION.SDK_INT,
): Boolean = missingPermissions(context, sdkInt).isEmpty()
