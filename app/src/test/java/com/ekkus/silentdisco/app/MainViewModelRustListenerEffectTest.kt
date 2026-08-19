package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.permissions.AppPermission
import com.ekkus.silentdisco.core.permissions.PermissionCatalogue
import com.ekkus.silentdisco.core.rust.RecordedCompletion
import com.ekkus.silentdisco.core.rust.RecordedFailure
import com.ekkus.silentdisco.core.transport.BleOperationResult
import com.ekkus.silentdisco.core.transport.MdnsOperationResult
import com.ekkus.silentdisco.core.transport.MdnsSessionAdvertisement
import com.ekkus.silentdisco.core.uniffi.FfiApprovalMode
import com.ekkus.silentdisco.core.uniffi.FfiAudioSource
import com.ekkus.silentdisco.core.uniffi.FfiCoreNotification
import com.ekkus.silentdisco.core.uniffi.FfiPlatformCompletion
import com.ekkus.silentdisco.core.uniffi.FfiPlatformEffect
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Before
import org.junit.Test

/**
 * Drives `MainViewModelRustListener.kt`'s `executeRustListenerPlatformEffect`
 * end-to-end through the real production dispatch path, mirroring
 * [MainViewModelRustHostEffectTest]'s approach on the listener side -- see
 * that class's doc comment for the rationale.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class MainViewModelRustListenerEffectTest {
    private val dispatcher = StandardTestDispatcher()

    @Before
    fun setUpMainDispatcher() {
        Dispatchers.setMain(dispatcher)
    }

    @After
    fun resetMainDispatcher() {
        Dispatchers.resetMain()
    }

    private fun grantListenerTransportPermissions(viewModel: MainViewModel) {
        val permissions = PermissionCatalogue.wifiDirectPermissions() +
            PermissionCatalogue.bluetoothPermissions().filter { it != AppPermission.BluetoothAdvertise }
        permissions.forEach { viewModel.updatePermission(it.androidPermission, true) }
    }

    @Test
    fun startDiscoveryWithoutNearbyPermissionsStillUsesMdns() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.ensureRustListenerCore()
        advanceUntilIdle()

        harness.listenerCoreController.emit(
            FfiCoreNotification.PlatformEffect(
                FfiPlatformEffect.StartDiscovery(operationId = "op-mdns-only", scanWindowMs = 3_000uL),
            ),
        )
        advanceUntilIdle()

        assertThat(harness.mdnsService.startCallCount).isEqualTo(1)
        assertThat(harness.bleService.startScanningCallCount).isEqualTo(0)
        assertThat(harness.wifiDirectService.discoverPeersCallCount).isEqualTo(0)
        assertThat(harness.listenerCoreController.platformOperationSucceededCalls).containsExactly(
            RecordedCompletion("op-mdns-only", FfiPlatformCompletion.DiscoveryStarted),
        )
    }

    @Test
    fun startDiscoveryFailsWithoutNearbyPermissionsWhenMdnsIsAlsoUnavailable() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.mdnsService.startResult = MdnsOperationResult.failed("Android NSD service is unavailable")
        harness.viewModel.ensureRustListenerCore()
        advanceUntilIdle()

        harness.listenerCoreController.emit(
            FfiCoreNotification.PlatformEffect(
                FfiPlatformEffect.StartDiscovery(operationId = "op-no-backend", scanWindowMs = 3_000uL),
            ),
        )
        advanceUntilIdle()

        assertThat(harness.listenerCoreController.platformOperationFailedCalls).containsExactly(
            RecordedFailure("op-no-backend", "Android NSD service is unavailable", true),
        )
    }

    @Test
    fun startDiscoverySucceedsAndStartsBleScanAndWifiDirectDiscovery() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        grantListenerTransportPermissions(harness.viewModel)
        harness.viewModel.ensureRustListenerCore()
        advanceUntilIdle()

        harness.listenerCoreController.emit(
            FfiCoreNotification.PlatformEffect(
                FfiPlatformEffect.StartDiscovery(operationId = "op-start", scanWindowMs = 3_000uL),
            ),
        )
        advanceUntilIdle()

        assertThat(harness.mdnsService.startCallCount).isEqualTo(1)
        assertThat(harness.bleService.startScanningCallCount).isEqualTo(1)
        assertThat(harness.wifiDirectService.discoverPeersCallCount).isEqualTo(1)
        assertThat(harness.listenerCoreController.platformOperationSucceededCalls).containsExactly(
            RecordedCompletion("op-start", FfiPlatformCompletion.DiscoveryStarted),
        )
    }

    @Test
    fun mdnsKeepsDiscoveryAvailableWhenBleScanFails() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        grantListenerTransportPermissions(harness.viewModel)
        harness.bleService.startScanningResult = BleOperationResult.failed("BLE scanner unavailable on this device")
        harness.viewModel.ensureRustListenerCore()
        advanceUntilIdle()

        harness.listenerCoreController.emit(
            FfiCoreNotification.PlatformEffect(
                FfiPlatformEffect.StartDiscovery(operationId = "op-ble-fail", scanWindowMs = 3_000uL),
            ),
        )
        advanceUntilIdle()

        assertThat(harness.wifiDirectService.discoverPeersCallCount).isEqualTo(0)
        assertThat(harness.listenerCoreController.platformOperationSucceededCalls).containsExactly(
            RecordedCompletion("op-ble-fail", FfiPlatformCompletion.DiscoveryStarted),
        )
    }

    @Test
    fun discoveryFailsWhenBothMdnsAndBleFail() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        grantListenerTransportPermissions(harness.viewModel)
        harness.mdnsService.startResult = MdnsOperationResult.failed("NSD unavailable")
        harness.bleService.startScanningResult = BleOperationResult.failed("BLE scanner unavailable on this device")
        harness.viewModel.ensureRustListenerCore()
        advanceUntilIdle()

        harness.listenerCoreController.emit(
            FfiCoreNotification.PlatformEffect(
                FfiPlatformEffect.StartDiscovery(operationId = "op-all-fail", scanWindowMs = 3_000uL),
            ),
        )
        advanceUntilIdle()

        assertThat(harness.listenerCoreController.platformOperationFailedCalls).containsExactly(
            RecordedFailure("op-all-fail", "BLE scanner unavailable on this device", true),
        )
        assertThat(harness.wifiDirectService.discoverPeersCallCount).isEqualTo(0)
    }

    @Test
    fun stopDiscoveryStopsBleScanAndWifiDirectDiscoveryAndReportsSuccess() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.ensureRustListenerCore()
        advanceUntilIdle()

        harness.listenerCoreController.emit(
            FfiCoreNotification.PlatformEffect(FfiPlatformEffect.StopDiscovery(operationId = "op-stop")),
        )
        advanceUntilIdle()

        assertThat(harness.mdnsService.stopCallCount).isEqualTo(1)
        assertThat(harness.bleService.stopScanningCallCount).isEqualTo(1)
        // wifiDirectService.cancelDiscovery() is SessionTransport's default
        // (`= stop()`), so a real stop() call is the real, observable fact.
        assertThat(harness.wifiDirectService.stopCallCount).isEqualTo(1)
        assertThat(harness.listenerCoreController.platformOperationSucceededCalls).containsExactly(
            RecordedCompletion("op-stop", FfiPlatformCompletion.DiscoveryStopped),
        )
    }

    @Test
    fun establishNetworkWithMdnsEndpointOpensRustTransportAndSendsJoinBeforeCompleting() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        val session = SessionInfo(
            id = "session-1",
            name = "Patio Mix",
            hostDeviceName = "host-1",
            approvalMode = ApprovalMode.MANUAL,
            inviteCodeRequired = false,
        )
        harness.viewModel._uiState.value = harness.viewModel._uiState.value.copy(discoveredSessions = listOf(session))
        harness.viewModel.ensureRustListenerCore()
        advanceUntilIdle()

        harness.listenerCoreController.emit(
            FfiCoreNotification.PlatformEffect(
                FfiPlatformEffect.EstablishNetwork(
                    operationId = "op-known",
                    sessionId = "session-1",
                    address = "192.168.1.50",
                    controlPort = 41_000u,
                    syncPort = 41_001u,
                    audioPort = 41_002u,
                ),
            ),
        )
        advanceUntilIdle()

        assertThat(harness.listenerTransport.connectCalls).hasSize(1)
        assertThat(harness.listenerTransport.connectCalls.single().rawEndpoint).contains("192.168.1.50")
        assertThat(harness.listenerTransport.joinRequestCalls).containsExactly("This Android Listener" to null)
        assertThat(harness.wifiDirectService.connectToSessionCalls).isEmpty()
        assertThat(harness.listenerCoreController.platformOperationSucceededCalls).containsExactly(
            RecordedCompletion(
                "op-known",
                FfiPlatformCompletion.NetworkEndpointReady(
                    address = "192.168.1.50",
                    controlPort = 41_000u,
                    syncPort = 41_001u,
                    audioPort = 41_002u,
                ),
            ),
        )
    }

    @Test
    fun establishNetworkFailsWhenTheSelectedSessionHasDisappeared() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.ensureRustListenerCore()
        advanceUntilIdle()

        harness.listenerCoreController.emit(
            FfiCoreNotification.PlatformEffect(
                FfiPlatformEffect.EstablishNetwork(
                    operationId = "op-gone",
                    sessionId = "session-not-discovered",
                    address = null,
                    controlPort = null,
                    syncPort = null,
                    audioPort = null,
                ),
            ),
        )
        advanceUntilIdle()

        assertThat(harness.listenerCoreController.platformOperationFailedCalls).containsExactly(
            RecordedFailure("op-gone", "Selected session disappeared before establishment", false),
        )
    }

    @Test
    fun establishNetworkForAKnownSessionConnectsThroughWifiDirectAndDefersCompletion() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        val session = SessionInfo(
            id = "session-1",
            name = "Patio Mix",
            hostDeviceName = "host-1",
            approvalMode = ApprovalMode.MANUAL,
            inviteCodeRequired = false,
        )
        harness.viewModel._uiState.value = harness.viewModel._uiState.value.copy(discoveredSessions = listOf(session))
        harness.viewModel.ensureRustListenerCore()
        advanceUntilIdle()

        harness.listenerCoreController.emit(
            FfiCoreNotification.PlatformEffect(
                FfiPlatformEffect.EstablishNetwork(
                    operationId = "op-connect",
                    sessionId = "session-1",
                    address = null,
                    controlPort = null,
                    syncPort = null,
                    audioPort = null,
                ),
            ),
        )
        advanceUntilIdle()

        assertThat(harness.wifiDirectService.connectToSessionCalls).containsExactly(session)
        // Completion is deferred to completeRustListenerNetworkEstablishment,
        // driven by a later Wi-Fi Direct CONNECTED snapshot -- unreachable
        // here without a real bound native listener transport, so this
        // asserts the real synchronous fact instead: nothing has been
        // reported back yet, success or failure.
        assertThat(harness.listenerCoreController.platformOperationSucceededCalls).isEmpty()
        assertThat(harness.listenerCoreController.platformOperationFailedCalls).isEmpty()
    }

    @Test
    fun releaseNetworkStopsWifiDirectAndReportsSuccess() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.ensureRustListenerCore()
        advanceUntilIdle()

        harness.listenerCoreController.emit(
            FfiCoreNotification.PlatformEffect(FfiPlatformEffect.ReleaseNetwork(operationId = "op-release")),
        )
        advanceUntilIdle()

        assertThat(harness.listenerTransport.disconnectCalls).containsExactly("Listener released network")
        assertThat(harness.wifiDirectService.stopCallCount).isEqualTo(1)
        assertThat(harness.listenerCoreController.platformOperationSucceededCalls).containsExactly(
            RecordedCompletion("op-release", FfiPlatformCompletion.NetworkReleased),
        )
    }

    @Test
    fun releasedEndpointConnectionCanJoinAnotherMdnsSessionWithoutClosingTheEventStream() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        val session = SessionInfo(
            id = "session-1",
            name = "Patio Mix",
            hostDeviceName = "host-1",
            approvalMode = ApprovalMode.MANUAL,
            inviteCodeRequired = false,
        )
        harness.viewModel._uiState.value = harness.viewModel._uiState.value.copy(discoveredSessions = listOf(session))
        harness.viewModel.ensureRustListenerCore()
        advanceUntilIdle()

        fun establish(operationId: String) {
            harness.listenerCoreController.emit(
                FfiCoreNotification.PlatformEffect(
                    FfiPlatformEffect.EstablishNetwork(
                        operationId = operationId,
                        sessionId = "session-1",
                        address = "192.168.1.50",
                        controlPort = 41_000u,
                        syncPort = 41_001u,
                        audioPort = 41_002u,
                    ),
                ),
            )
        }

        establish("op-first")
        advanceUntilIdle()
        harness.listenerCoreController.emit(
            FfiCoreNotification.PlatformEffect(FfiPlatformEffect.ReleaseNetwork(operationId = "op-release")),
        )
        advanceUntilIdle()
        establish("op-second")
        advanceUntilIdle()

        assertThat(harness.listenerTransport.disconnectCalls).containsExactly("Listener released network")
        assertThat(harness.listenerTransport.connectCalls).hasSize(2)
        assertThat(harness.listenerTransport.joinRequestCalls).hasSize(2)
        assertThat(harness.listenerTransport.closeCallCount).isEqualTo(0)
    }

    @Test
    fun mdnsAdvertisementReplacesBleFallbackForTheSameSessionId() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.ensureRustListenerCore()
        advanceUntilIdle()
        harness.bleService.setSessions(
            listOf(
                SessionInfo(
                    id = "session-1",
                    name = "BLE Patio",
                    hostDeviceName = "ble-host",
                    approvalMode = ApprovalMode.MANUAL,
                    inviteCodeRequired = false,
                ),
            ),
        )
        advanceUntilIdle()
        harness.mdnsService.setSessions(
            listOf(
                MdnsSessionAdvertisement(
                    sessionId = "session-1",
                    hostDeviceId = "desktop-host",
                    sessionName = "Desktop Patio",
                    approvalMode = ApprovalMode.INVITE_CODE,
                    protocolVersion = 2,
                    address = "192.168.1.50",
                    controlPort = 41_100,
                    syncPort = 41_101,
                    audioPort = 41_102,
                ),
            ),
        )
        advanceUntilIdle()

        val latest = harness.listenerCoreController.submitSessionDiscoveredCalls.last()
        assertThat(latest.sessionId).isEqualTo("session-1")
        assertThat(latest.hostDeviceId).isEqualTo("desktop-host")
        assertThat(latest.address).isEqualTo("192.168.1.50")
        assertThat(latest.controlPort).isEqualTo(41_100u.toUShort())
    }

    @Test
    fun platformEffectsOutsideAndroidListenerBlock13AreEachRejectedWithTheRealMessage() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.ensureRustListenerCore()
        advanceUntilIdle()

        val effectsByOperationId = linkedMapOf(
            "op-start-advertising" to FfiPlatformEffect.StartAdvertising(
                operationId = "op-start-advertising",
                sessionId = "session-1",
                hostDeviceId = "host-1",
                sessionName = "Patio Mix",
                approvalMode = FfiApprovalMode.MANUAL,
            ),
            "op-stop-advertising" to FfiPlatformEffect.StopAdvertising(operationId = "op-stop-advertising"),
            "op-caps" to FfiPlatformEffect.RequestCapabilities(operationId = "op-caps", capabilities = emptyList()),
            "op-prepare-audio" to FfiPlatformEffect.PrepareAudioSource(
                operationId = "op-prepare-audio",
                source = FfiAudioSource(
                    sourceId = "src-1",
                    displayName = "Track",
                    sizeBytes = null,
                    durationMs = null,
                ),
            ),
            "op-audio-start" to FfiPlatformEffect.StartAudioOutput(
                operationId = "op-audio-start",
                sampleRateHz = 48_000u,
                channels = 2u,
            ),
            "op-audio-stop" to FfiPlatformEffect.StopAudioOutput(operationId = "op-audio-stop"),
            "op-share-diag" to FfiPlatformEffect.ShareDiagnostics(
                operationId = "op-share-diag",
                exportId = "export-1",
            ),
        )

        effectsByOperationId.values.forEach { effect ->
            harness.listenerCoreController.emit(FfiCoreNotification.PlatformEffect(effect))
        }
        advanceUntilIdle()

        val expected = effectsByOperationId.keys.map { operationId ->
            RecordedFailure(operationId, "Platform effect is outside Android listener Block 13", false)
        }
        assertThat(harness.listenerCoreController.platformOperationFailedCalls).containsExactlyElementsIn(expected)
    }
}
