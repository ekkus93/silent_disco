package com.ekkus.silentdisco.app.controller

import com.ekkus.silentdisco.core.rust.FakeListenerCoreController
import com.ekkus.silentdisco.core.transport.BleOperationResult
import com.ekkus.silentdisco.core.transport.FakeBleTransport
import com.ekkus.silentdisco.core.transport.FakeMdnsTransport
import com.ekkus.silentdisco.core.transport.FakeSessionTransport
import com.ekkus.silentdisco.core.transport.MdnsOperationResult
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runTest
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class DiscoveryControllerTest {
    private val ble = FakeBleTransport()
    private val mdns = FakeMdnsTransport()
    private val wifiDirect = FakeSessionTransport()
    private var reconcileProbes = 0

    private fun TestScope.controller(): Pair<DiscoveryController, CoroutineScope> {
        val parent = CoroutineScope(SupervisorJob() + StandardTestDispatcher(testScheduler))
        val core = FakeListenerCoreController()
        val discovery = DiscoveryController(parent, ble, mdns, wifiDirect) {
            reconcileProbes += 1
            core
        }
        return discovery to parent
    }

    @Test
    fun withoutNearbyPermissionsOnlyMdnsStarts() = runTest {
        val (discovery, parent) = controller()

        assertThat(discovery.start(hasNearbyPermissions = false)).isNull()
        assertThat(mdns.startCallCount).isEqualTo(1)
        assertThat(ble.startScanningCallCount).isEqualTo(0)
        assertThat(wifiDirect.discoverPeersCallCount).isEqualTo(0)

        mdns.startResult = MdnsOperationResult.failed("no multicast lock")
        assertThat(discovery.start(hasNearbyPermissions = false)).isEqualTo("no multicast lock")
        parent.cancel()
    }

    @Test
    fun withPermissionsBleAndWifiDirectStartAndOneRadioIsEnough() = runTest {
        val (discovery, parent) = controller()
        mdns.startResult = MdnsOperationResult.failed("no multicast lock")

        assertThat(discovery.start(hasNearbyPermissions = true)).isNull()
        assertThat(ble.startScanningCallCount).isEqualTo(1)
        assertThat(wifiDirect.discoverPeersCallCount).isEqualTo(1)

        ble.startScanningResult = BleOperationResult.failed("bluetooth off")
        assertThat(discovery.start(hasNearbyPermissions = true)).isEqualTo("bluetooth off")
        assertThat(wifiDirect.discoverPeersCallCount).isEqualTo(1)
        parent.cancel()
    }

    @Test
    fun stopStopsEveryRadio() = runTest {
        val (discovery, parent) = controller()

        assertThat(discovery.stop()).isNull()

        assertThat(mdns.stopCallCount).isEqualTo(1)
        assertThat(ble.stopScanningCallCount).isEqualTo(1)
        assertThat(wifiDirect.stopCallCount).isEqualTo(1)
        parent.cancel()
    }

    @Test
    fun theScanWindowReconcilesOnceItEndsUnlessCancelled() = runTest {
        val (discovery, parent) = controller()

        discovery.startScanWindow(4_000L)
        advanceTimeBy(3_999L)
        assertThat(reconcileProbes).isEqualTo(0)
        advanceTimeBy(2L)
        assertThat(reconcileProbes).isEqualTo(1)

        discovery.startScanWindow(4_000L)
        discovery.cancelScanWindow()
        advanceTimeBy(5_000L)
        assertThat(reconcileProbes).isEqualTo(1)
        parent.cancel()
    }
}
