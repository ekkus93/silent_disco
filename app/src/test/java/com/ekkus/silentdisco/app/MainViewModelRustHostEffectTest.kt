package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.permissions.PermissionCatalogue
import com.ekkus.silentdisco.core.rust.RecordedCompletion
import com.ekkus.silentdisco.core.rust.RecordedFailure
import com.ekkus.silentdisco.core.transport.BleOperationResult
import com.ekkus.silentdisco.core.transport.TransportOperationResult
import com.ekkus.silentdisco.core.uniffi.FfiApprovalMode
import com.ekkus.silentdisco.core.uniffi.FfiAudioSource
import com.ekkus.silentdisco.core.uniffi.FfiCoreNotification
import com.ekkus.silentdisco.core.uniffi.FfiDeliveryReport
import com.ekkus.silentdisco.core.uniffi.FfiPlatformCompletion
import com.ekkus.silentdisco.core.uniffi.FfiPlatformEffect
import com.ekkus.silentdisco.core.uniffi.FfiStorageEffect
import com.ekkus.silentdisco.core.uniffi.FfiTransportEffect
import com.ekkus.silentdisco.core.uniffi.FfiTransportState
import com.ekkus.silentdisco.core.uniffi.FfiTuningSettings
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Before
import org.junit.Test

/**
 * Drives `MainViewModelRustHost.kt`'s effect-runner functions
 * (`executeRustPlatformEffect`/`executeRustTransportEffect`/
 * `executeRustStorageEffect`) end-to-end through the real production
 * dispatch path -- `ensureRustHostCore()` wires a real collector onto
 * [com.ekkus.silentdisco.core.rust.FakeHostCoreController.notifications],
 * exactly as it would a real Rust actor's notifications, and each test
 * pushes a notification through that same pipe via `emit` and asserts on
 * exactly what got reported back. Nothing here synthesizes a success --
 * every assertion is either a real fake-collaborator recorded call or a
 * real production failure message.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class MainViewModelRustHostEffectTest {
    private val dispatcher = StandardTestDispatcher()

    @Before
    fun setUpMainDispatcher() {
        Dispatchers.setMain(dispatcher)
    }

    @After
    fun resetMainDispatcher() {
        Dispatchers.resetMain()
    }

    private fun grantHostTransportPermissions(viewModel: MainViewModel) {
        (PermissionCatalogue.wifiDirectPermissions() + PermissionCatalogue.bluetoothPermissions()).forEach {
            viewModel.updatePermission(it.androidPermission, true)
        }
    }

    /**
     * `HostTransportController`'s send/broadcast methods (the real,
     * unbound production class used by these tests -- see
     * [newTestMainViewModelHarness]'s doc comment) genuinely hop onto the
     * real `Dispatchers.IO` thread pool, even with no bound handle, just to
     * evaluate a null check. That real thread is outside this test's
     * virtual [dispatcher], so a single `advanceUntilIdle()` can race the
     * real background thread posting its continuation back onto
     * `Dispatchers.Main`. Draining the virtual scheduler between short real
     * waits lets that real hop actually land before asserting -- this is a
     * genuine cross-thread wait, not a virtual-time shortcut.
     */
    private fun TestScope.awaitRealDispatchHop(condition: () -> Boolean) {
        repeat(200) {
            advanceUntilIdle()
            if (condition()) return
            Thread.sleep(5)
        }
        check(condition()) { "Condition was not met after waiting for a real Dispatchers.IO hop to complete" }
    }

    private fun startAdvertisingEffect(operationId: String) = FfiPlatformEffect.StartAdvertising(
        operationId = operationId,
        sessionId = "session-1",
        hostDeviceId = "host-1",
        sessionName = "Patio Mix",
        approvalMode = FfiApprovalMode.MANUAL,
    )

    @Test
    fun startAdvertisingFailsWithoutNearbyConnectivityPermissions() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.ensureRustHostCore()
        advanceUntilIdle()

        harness.hostCoreController.emit(FfiCoreNotification.PlatformEffect(startAdvertisingEffect("op-perm")))
        advanceUntilIdle()

        assertThat(harness.hostCoreController.platformOperationFailedCalls).containsExactly(
            RecordedFailure("op-perm", "Missing nearby connectivity permissions for advertising", true),
        )
        assertThat(harness.bleService.startAdvertisingCalls).isEmpty()
        assertThat(harness.wifiDirectService.startHostCalls).isEmpty()
    }

    @Test
    fun startAdvertisingFailsWhenBleAdvertisingFails() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        grantHostTransportPermissions(harness.viewModel)
        harness.bleService.startAdvertisingResult =
            BleOperationResult.failed("BLE advertiser unavailable on this device")
        harness.viewModel.ensureRustHostCore()
        advanceUntilIdle()

        harness.hostCoreController.emit(FfiCoreNotification.PlatformEffect(startAdvertisingEffect("op-ble")))
        advanceUntilIdle()

        assertThat(harness.hostCoreController.platformOperationFailedCalls).containsExactly(
            RecordedFailure("op-ble", "BLE advertiser unavailable on this device", true),
        )
        assertThat(harness.wifiDirectService.startHostCalls).isEmpty()
    }

    @Test
    fun startAdvertisingFailsAndStopsBleWhenWifiDirectHostFails() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        grantHostTransportPermissions(harness.viewModel)
        harness.wifiDirectService.startHostResult =
            TransportOperationResult.failed("Wi-Fi Direct manager unavailable on this device")
        harness.viewModel.ensureRustHostCore()
        advanceUntilIdle()

        harness.hostCoreController.emit(FfiCoreNotification.PlatformEffect(startAdvertisingEffect("op-wifi")))
        advanceUntilIdle()

        assertThat(harness.hostCoreController.platformOperationFailedCalls).containsExactly(
            RecordedFailure("op-wifi", "Wi-Fi Direct manager unavailable on this device", true),
        )
        assertThat(harness.bleService.stopAdvertisingCallCount).isEqualTo(1)
    }

    @Test
    fun startAdvertisingFailsAndStopsBleWhenWifiDirectHostThrows() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        grantHostTransportPermissions(harness.viewModel)
        harness.wifiDirectService.startHostFailure = IllegalStateException("wifi direct exploded")
        harness.viewModel.ensureRustHostCore()
        advanceUntilIdle()

        harness.hostCoreController.emit(FfiCoreNotification.PlatformEffect(startAdvertisingEffect("op-wifi-throw")))
        advanceUntilIdle()

        assertThat(harness.hostCoreController.platformOperationFailedCalls).containsExactly(
            RecordedFailure("op-wifi-throw", "wifi direct exploded", true),
        )
        assertThat(harness.bleService.stopAdvertisingCallCount).isEqualTo(1)
    }

    @Test
    fun startAdvertisingSucceedsThroughBleAndWifiDirectUpToTheAsyncBindStep() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        grantHostTransportPermissions(harness.viewModel)
        harness.viewModel.ensureRustHostCore()
        advanceUntilIdle()

        harness.hostCoreController.emit(FfiCoreNotification.PlatformEffect(startAdvertisingEffect("op-success")))
        advanceUntilIdle()

        // AdvertisingStarted is only reported once completeRustHostAdvertising
        // observes the Wi-Fi Direct transport reach ADVERTISING with a real
        // address and binds the real (native) host transport -- unreachable
        // from a JVM test. This asserts the synchronous portion of the real
        // effect instead: BLE and Wi-Fi Direct were both genuinely asked to
        // start, and neither reported a failure.
        assertThat(harness.bleService.startAdvertisingCalls).hasSize(1)
        assertThat(harness.wifiDirectService.startHostCalls).hasSize(1)
        assertThat(harness.hostCoreController.platformOperationFailedCalls).isEmpty()
        assertThat(harness.hostCoreController.platformOperationSucceededCalls).isEmpty()
    }

    @Test
    fun stopAdvertisingRunsRealCleanupAndReportsSuccess() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.ensureRustHostCore()
        advanceUntilIdle()

        harness.hostCoreController.emit(
            FfiCoreNotification.PlatformEffect(FfiPlatformEffect.StopAdvertising(operationId = "op-stop")),
        )
        advanceUntilIdle()

        assertThat(harness.bleService.stopCallCount).isEqualTo(1)
        assertThat(harness.wifiDirectService.stopCallCount).isEqualTo(1)
        assertThat(harness.hostCoreController.transportStateChangedCalls).contains(FfiTransportState.IDLE)
        assertThat(harness.hostCoreController.platformOperationSucceededCalls).containsExactly(
            RecordedCompletion("op-stop", FfiPlatformCompletion.AdvertisingStopped),
        )
    }

    @Test
    fun requestCapabilitiesEffectIsRejectedAsOutsideHostCreation() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.ensureRustHostCore()
        advanceUntilIdle()

        harness.hostCoreController.emit(
            FfiCoreNotification.PlatformEffect(
                FfiPlatformEffect.RequestCapabilities(operationId = "op-caps", capabilities = listOf("audio")),
            ),
        )
        advanceUntilIdle()

        assertThat(harness.hostCoreController.platformOperationFailedCalls).containsExactly(
            RecordedFailure("op-caps", "Android capability requests must be resolved before host creation", true),
        )
    }

    @Test
    fun platformEffectsOutsideAndroidHostBlock12AreEachRejectedWithTheRealMessage() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.ensureRustHostCore()
        advanceUntilIdle()

        val effectsByOperationId = linkedMapOf(
            "op-discovery-start" to FfiPlatformEffect.StartDiscovery(
                operationId = "op-discovery-start",
                scanWindowMs = 3_000uL,
            ),
            "op-discovery-stop" to FfiPlatformEffect.StopDiscovery(operationId = "op-discovery-stop"),
            "op-establish" to FfiPlatformEffect.EstablishNetwork(
                operationId = "op-establish",
                sessionId = "session-1",
                address = null,
                controlPort = null,
                syncPort = null,
                audioPort = null,
            ),
            "op-release" to FfiPlatformEffect.ReleaseNetwork(operationId = "op-release"),
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
            harness.hostCoreController.emit(FfiCoreNotification.PlatformEffect(effect))
        }
        advanceUntilIdle()

        val expected = effectsByOperationId.keys.map { operationId ->
            RecordedFailure(operationId, "Platform effect is outside Android host Block 12", false)
        }
        assertThat(harness.hostCoreController.platformOperationFailedCalls).containsExactlyElementsIn(expected)
    }

    private fun deliverJoinApprovalEffect(operationId: String) = FfiTransportEffect.DeliverJoinApproval(
        operationId = operationId,
        requestId = "req-1",
        sessionId = "session-1",
        listenerId = "listener-1",
        trustedForFuture = false,
    )

    @Test
    fun deliverJoinApprovalWithNoBoundTransportReportsZeroPeersDelivered() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.ensureRustHostCore()
        advanceUntilIdle()

        harness.hostCoreController.emit(
            FfiCoreNotification.TransportEffect(deliverJoinApprovalEffect("op-approve")),
        )
        awaitRealDispatchHop { harness.hostCoreController.transportDeliveryCompletedCalls.isNotEmpty() }

        assertThat(harness.hostCoreController.transportDeliveryCompletedCalls).containsExactly(
            "op-approve" to FfiDeliveryReport(intendedPeers = 0u, successfulPeers = 0u, failedPeers = 0u),
        )
        // Zero peers actually reached means the join never really connected --
        // submitListenerConnected must not fire on an undelivered approval.
        assertThat(harness.hostCoreController.submitListenerConnectedCalls).isEmpty()
    }

    @Test
    fun deliverJoinRejectionWithNoBoundTransportReportsZeroPeersDelivered() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.ensureRustHostCore()
        advanceUntilIdle()

        harness.hostCoreController.emit(
            FfiCoreNotification.TransportEffect(
                FfiTransportEffect.DeliverJoinRejection(
                    operationId = "op-reject",
                    requestId = "req-1",
                    sessionId = "session-1",
                    listenerId = "listener-1",
                    reasonCode = "denied",
                ),
            ),
        )
        awaitRealDispatchHop { harness.hostCoreController.transportDeliveryCompletedCalls.isNotEmpty() }

        assertThat(harness.hostCoreController.transportDeliveryCompletedCalls).containsExactly(
            "op-reject" to FfiDeliveryReport(intendedPeers = 0u, successfulPeers = 0u, failedPeers = 0u),
        )
    }

    @Test
    fun disconnectListenerWithNoBoundTransportReportsZeroPeersDelivered() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.ensureRustHostCore()
        advanceUntilIdle()

        harness.hostCoreController.emit(
            FfiCoreNotification.TransportEffect(
                FfiTransportEffect.DisconnectListener(
                    operationId = "op-disconnect",
                    sessionId = "session-1",
                    listenerId = "listener-1",
                    reasonCode = "host_ended_session",
                ),
            ),
        )
        awaitRealDispatchHop { harness.hostCoreController.transportDeliveryCompletedCalls.isNotEmpty() }

        assertThat(harness.hostCoreController.transportDeliveryCompletedCalls).containsExactly(
            "op-disconnect" to FfiDeliveryReport(intendedPeers = 0u, successfulPeers = 0u, failedPeers = 0u),
        )
    }

    private val tuningSettings = FfiTuningSettings(
        syncSampleWindow = 12u,
        syncCadenceMs = 2_000uL,
        startupBufferMs = 400uL,
        latePacketThresholdMs = 40uL,
        hardResyncThresholdMs = 120uL,
        syncDriftThresholdMs = 18.0,
        scanWindowMs = 3_000uL,
    )

    @Test
    fun persistSettingsSuccessReportsSettingsSaved() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.ensureRustHostCore()
        advanceUntilIdle()

        harness.hostCoreController.emit(
            FfiCoreNotification.StorageEffect(
                FfiStorageEffect.PersistSettings(operationId = "op-settings", settings = tuningSettings),
            ),
        )
        advanceUntilIdle()

        assertThat(harness.domainStore.saveTuningCalls).hasSize(1)
        val saved = harness.domainStore.saveTuningCalls.single()
        assertThat(saved.syncSampleWindow).isEqualTo(12)
        assertThat(saved.syncCadenceMs).isEqualTo(2_000L)
        assertThat(saved.startupBufferMs).isEqualTo(400L)
        assertThat(harness.hostCoreController.settingsSavedCalls).containsExactly("op-settings")
        assertThat(harness.hostCoreController.storageOperationFailedCalls).isEmpty()
    }

    @Test
    fun persistSettingsFailureReportsStorageOperationFailedNotASwallowedException() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.domainStore.saveTuningFailure = IllegalStateException("disk full")
        harness.viewModel.ensureRustHostCore()
        advanceUntilIdle()

        harness.hostCoreController.emit(
            FfiCoreNotification.StorageEffect(
                FfiStorageEffect.PersistSettings(operationId = "op-settings-fail", settings = tuningSettings),
            ),
        )
        advanceUntilIdle()

        assertThat(harness.hostCoreController.storageOperationFailedCalls).containsExactly(
            RecordedFailure("op-settings-fail", "disk full", true),
        )
        assertThat(harness.hostCoreController.settingsSavedCalls).isEmpty()
    }

    @Test
    fun persistTrustedDeviceSuccessReportsTrustedDeviceUpdated() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.viewModel.ensureRustHostCore()
        advanceUntilIdle()

        harness.hostCoreController.emit(
            FfiCoreNotification.StorageEffect(
                FfiStorageEffect.PersistTrustedDevice(
                    operationId = "op-trust",
                    deviceId = "listener-1",
                    displayName = "Alex's phone",
                ),
            ),
        )
        advanceUntilIdle()

        assertThat(harness.domainStore.trustDeviceCalls).hasSize(1)
        val (deviceId, displayName, _) = harness.domainStore.trustDeviceCalls.single()
        assertThat(deviceId).isEqualTo("listener-1")
        assertThat(displayName).isEqualTo("Alex's phone")
        assertThat(harness.hostCoreController.trustedDeviceUpdatedCalls).containsExactly(
            "op-trust" to "listener-1",
        )
    }

    @Test
    fun persistTrustedDeviceFailureReportsStorageOperationFailedNotASwallowedException() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.domainStore.trustDeviceFailure = RuntimeException("db locked")
        harness.viewModel.ensureRustHostCore()
        advanceUntilIdle()

        harness.hostCoreController.emit(
            FfiCoreNotification.StorageEffect(
                FfiStorageEffect.PersistTrustedDevice(
                    operationId = "op-trust-fail",
                    deviceId = "listener-1",
                    displayName = "Alex's phone",
                ),
            ),
        )
        advanceUntilIdle()

        assertThat(harness.hostCoreController.storageOperationFailedCalls).containsExactly(
            RecordedFailure("op-trust-fail", "db locked", true),
        )
        assertThat(harness.hostCoreController.trustedDeviceUpdatedCalls).isEmpty()
    }
}
