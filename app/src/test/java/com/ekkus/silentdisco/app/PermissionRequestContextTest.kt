package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.permissions.AppPermission
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class PermissionRequestContextTest {
    @Test
    fun api29HostUsesLocationAndWifiWithoutRuntimeBluetoothPermissions() {
        val permissions = PermissionRequestContext.HOST_NEARBY.requiredPermissions(sdkInt = 29)

        assertThat(permissions).contains(AppPermission.FineLocation)
        assertThat(permissions).contains(AppPermission.WifiState)
        assertThat(permissions).contains(AppPermission.ChangeWifiState)
        assertThat(permissions).doesNotContain(AppPermission.BluetoothScan)
    }

    @Test
    fun api31ListenerUsesLocationAndListenerBluetoothPermissions() {
        val permissions = PermissionRequestContext.LISTENER_NEARBY.requiredPermissions(sdkInt = 31)

        assertThat(permissions).contains(AppPermission.FineLocation)
        assertThat(permissions).contains(AppPermission.BluetoothScan)
        assertThat(permissions).contains(AppPermission.BluetoothConnect)
        assertThat(permissions).doesNotContain(AppPermission.BluetoothAdvertise)
    }

    @Test
    fun api33HostUsesNearbyWifiAndAllHostBluetoothPermissions() {
        val permissions = PermissionRequestContext.HOST_NEARBY.requiredPermissions(sdkInt = 33)

        assertThat(permissions).contains(AppPermission.NearbyWifiDevices)
        assertThat(permissions).contains(AppPermission.BluetoothScan)
        assertThat(permissions).contains(AppPermission.BluetoothAdvertise)
        assertThat(permissions).contains(AppPermission.BluetoothConnect)
        assertThat(permissions).doesNotContain(AppPermission.FineLocation)
    }

    @Test
    fun documentPickerRequiresNoBroadMediaPermission() {
        assertThat(PermissionRequestContext.AUDIO_FILE.requiredPermissions(sdkInt = 36)).isEmpty()
    }
}
