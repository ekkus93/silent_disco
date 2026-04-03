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
    fun requiredPermissions(): List<AppPermission> = buildList {
        add(AppPermission.NearbyWifiDevices)
        add(AppPermission.WifiState)
        add(AppPermission.ChangeWifiState)
        add(AppPermission.FineLocation)
        add(AppPermission.BluetoothScan)
        add(AppPermission.BluetoothAdvertise)
        add(AppPermission.BluetoothConnect)
        add(AppPermission.ReadMediaAudio)
    }
}
