package com.ekkus.silentdisco.core.permissions

import android.Manifest
import android.os.Build

enum class AppPermission(val androidPermission: String) {
    NearbyWifiDevices("android.permission.NEARBY_WIFI_DEVICES"),
    WifiState(Manifest.permission.ACCESS_WIFI_STATE),
    ChangeWifiState(Manifest.permission.CHANGE_WIFI_STATE),
    FineLocation(Manifest.permission.ACCESS_FINE_LOCATION),
    BluetoothScan("android.permission.BLUETOOTH_SCAN"),
    BluetoothAdvertise("android.permission.BLUETOOTH_ADVERTISE"),
    BluetoothConnect("android.permission.BLUETOOTH_CONNECT"),
    ReadMediaAudio(
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            Manifest.permission.READ_MEDIA_AUDIO
        } else {
            Manifest.permission.READ_EXTERNAL_STORAGE
        },
    ),
}

data class PermissionState(
    val permission: AppPermission,
    val granted: Boolean,
)

object PermissionCatalogue {
    fun requiredPermissions(sdkInt: Int = Build.VERSION.SDK_INT): List<AppPermission> = buildList {
        addAll(wifiDirectPermissions(sdkInt))

        // Bluetooth runtime permissions require API 31+
        if (sdkInt >= Build.VERSION_CODES.S) {
            add(AppPermission.BluetoothScan)
            add(AppPermission.BluetoothAdvertise)
            add(AppPermission.BluetoothConnect)
        }

        add(AppPermission.ReadMediaAudio)
    }

    fun wifiDirectPermissions(sdkInt: Int = Build.VERSION.SDK_INT): List<AppPermission> = buildList {
        // NEARBY_WIFI_DEVICES is the API-33+ replacement for FineLocation on
        // paper, but real-device testing (Samsung SM-A546E, Android 16)
        // showed WifiP2pManager.createGroup still fails internally
        // (ActionListener#onFailure reason=0/ERROR) without FineLocation
        // ALSO granted, even with NEARBY_WIFI_DEVICES granted -- some
        // AOSP/OEM-internal WifiP2pManager/WifiPermissionsUtil codepath
        // apparently still checks it directly. Request both rather than
        // either/or.
        if (sdkInt >= Build.VERSION_CODES.TIRAMISU) {
            add(AppPermission.NearbyWifiDevices)
        }
        add(AppPermission.FineLocation)
        add(AppPermission.WifiState)
        add(AppPermission.ChangeWifiState)
    }

    fun bluetoothPermissions(sdkInt: Int = Build.VERSION.SDK_INT): List<AppPermission> = buildList {
        if (sdkInt >= Build.VERSION_CODES.S) {
            add(AppPermission.BluetoothScan)
            add(AppPermission.BluetoothAdvertise)
            add(AppPermission.BluetoothConnect)
        }
    }
}
