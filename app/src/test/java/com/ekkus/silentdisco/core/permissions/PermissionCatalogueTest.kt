package com.ekkus.silentdisco.core.permissions

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class PermissionCatalogueTest {

    @Test
    fun requiredPermissions_api29_doesNotIncludeNearbyWifiDevices() {
        val perms = PermissionCatalogue.requiredPermissions(sdkInt = 29)
        assertThat(perms).doesNotContain(AppPermission.NearbyWifiDevices)
    }

    @Test
    fun requiredPermissions_api29_includesFineLocation() {
        val perms = PermissionCatalogue.requiredPermissions(sdkInt = 29)
        assertThat(perms).contains(AppPermission.FineLocation)
    }

    @Test
    fun requiredPermissions_api29_doesNotIncludeBluetoothRuntimePermissions() {
        val perms = PermissionCatalogue.requiredPermissions(sdkInt = 29)
        assertThat(perms).doesNotContain(AppPermission.BluetoothScan)
        assertThat(perms).doesNotContain(AppPermission.BluetoothAdvertise)
        assertThat(perms).doesNotContain(AppPermission.BluetoothConnect)
    }

    @Test
    fun requiredPermissions_api31_includesBluetoothRuntimePermissions() {
        val perms = PermissionCatalogue.requiredPermissions(sdkInt = 31)
        assertThat(perms).contains(AppPermission.BluetoothScan)
        assertThat(perms).contains(AppPermission.BluetoothAdvertise)
        assertThat(perms).contains(AppPermission.BluetoothConnect)
    }

    @Test
    fun requiredPermissions_api31_doesNotIncludeNearbyWifiDevices() {
        val perms = PermissionCatalogue.requiredPermissions(sdkInt = 31)
        assertThat(perms).doesNotContain(AppPermission.NearbyWifiDevices)
    }

    @Test
    fun requiredPermissions_api33_includesNearbyWifiDevices() {
        val perms = PermissionCatalogue.requiredPermissions(sdkInt = 33)
        assertThat(perms).contains(AppPermission.NearbyWifiDevices)
    }

    @Test
    fun requiredPermissions_api33_alsoIncludesFineLocation() {
        // Real-device testing (Samsung SM-A546E, Android 16) showed
        // WifiP2pManager.createGroup still requires FineLocation internally
        // even with NearbyWifiDevices granted -- see wifiDirectPermissions.
        val perms = PermissionCatalogue.requiredPermissions(sdkInt = 33)
        assertThat(perms).contains(AppPermission.FineLocation)
    }

    @Test
    fun requiredPermissions_api33_includesBluetoothRuntimePermissions() {
        val perms = PermissionCatalogue.requiredPermissions(sdkInt = 33)
        assertThat(perms).contains(AppPermission.BluetoothScan)
        assertThat(perms).contains(AppPermission.BluetoothAdvertise)
        assertThat(perms).contains(AppPermission.BluetoothConnect)
    }

    @Test
    fun requiredPermissions_alwaysIncludesWifiStateAndChangeWifiState() {
        for (sdk in listOf(29, 30, 31, 32, 33)) {
            val perms = PermissionCatalogue.requiredPermissions(sdkInt = sdk)
            assertThat(perms).contains(AppPermission.WifiState)
            assertThat(perms).contains(AppPermission.ChangeWifiState)
        }
    }

    @Test
    fun requiredPermissions_alwaysIncludesReadMediaAudio() {
        for (sdk in listOf(29, 30, 31, 32, 33)) {
            val perms = PermissionCatalogue.requiredPermissions(sdkInt = sdk)
            assertThat(perms).contains(AppPermission.ReadMediaAudio)
        }
    }

    @Test
    fun wifiDirectPermissions_api30_usesFineLocation() {
        val perms = PermissionCatalogue.wifiDirectPermissions(sdkInt = 30)
        assertThat(perms).contains(AppPermission.FineLocation)
        assertThat(perms).doesNotContain(AppPermission.NearbyWifiDevices)
    }

    @Test
    fun wifiDirectPermissions_api33_requestsBothNearbyWifiAndFineLocation() {
        // NEARBY_WIFI_DEVICES is the documented API-33+ replacement for
        // FineLocation, but a physical device confirmed
        // WifiP2pManager.createGroup fails (reason=0/ERROR) without
        // FineLocation also granted -- request both, not either/or.
        val perms = PermissionCatalogue.wifiDirectPermissions(sdkInt = 33)
        assertThat(perms).contains(AppPermission.NearbyWifiDevices)
        assertThat(perms).contains(AppPermission.FineLocation)
    }

    @Test
    fun bluetoothPermissions_api30_isEmpty() {
        val perms = PermissionCatalogue.bluetoothPermissions(sdkInt = 30)
        assertThat(perms).isEmpty()
    }

    @Test
    fun bluetoothPermissions_api31_hasAllThree() {
        val perms = PermissionCatalogue.bluetoothPermissions(sdkInt = 31)
        assertThat(perms).containsExactly(
            AppPermission.BluetoothScan,
            AppPermission.BluetoothAdvertise,
            AppPermission.BluetoothConnect,
        )
    }
}
