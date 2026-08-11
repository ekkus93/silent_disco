package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.permissions.PermissionCatalogue
import com.ekkus.silentdisco.core.rust.RecordedFailure
import com.ekkus.silentdisco.core.uniffi.FfiApprovalMode
import com.ekkus.silentdisco.core.uniffi.FfiCoreNotification
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

@OptIn(ExperimentalCoroutinesApi::class)
class MainViewModelRustHostNotificationRecoveryTest {
    private val dispatcher = StandardTestDispatcher()

    @Before
    fun setUpMainDispatcher() {
        Dispatchers.setMain(dispatcher)
    }

    @After
    fun resetMainDispatcher() {
        Dispatchers.resetMain()
    }

    @Test
    fun unexpectedPlatformEffectFailureIsVisibleCorrelatedAndDoesNotKillCollector() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        (PermissionCatalogue.wifiDirectPermissions() + PermissionCatalogue.bluetoothPermissions()).forEach {
            harness.viewModel.updatePermission(it.androidPermission, true)
        }
        harness.bleService.startAdvertisingFailure = IllegalStateException("bluetooth stack exploded")
        harness.viewModel.ensureRustHostCore()
        advanceUntilIdle()

        harness.hostCoreController.emit(
            FfiCoreNotification.PlatformEffect(
                FfiPlatformEffect.StartAdvertising(
                    operationId = "op-crash",
                    sessionId = "session-1",
                    hostDeviceId = "host-1",
                    sessionName = "Patio Mix",
                    approvalMode = FfiApprovalMode.MANUAL,
                ),
            ),
        )
        advanceUntilIdle()

        val failureMessage = "Rust host platform effect failed unexpectedly: bluetooth stack exploded"
        assertThat(harness.hostCoreController.platformOperationFailedCalls).containsExactly(
            RecordedFailure("op-crash", failureMessage, false),
        )
        assertThat(harness.viewModel.uiState.value.lastError).isEqualTo(failureMessage)
        assertThat(harness.viewModel.uiState.value.hostDiagnostics.lastError).isEqualTo(failureMessage)
        assertThat(harness.viewModel.currentSessionId).isNull()
        assertThat(harness.viewModel.currentStreamId).isNull()
        assertThat(harness.bleService.stopAdvertisingCallCount).isEqualTo(1)
        assertThat(harness.wifiDirectService.stopCallCount).isEqualTo(1)

        harness.bleService.startAdvertisingFailure = null
        harness.hostCoreController.emit(
            FfiCoreNotification.PlatformEffect(
                FfiPlatformEffect.RequestCapabilities(
                    operationId = "op-after-crash",
                    capabilities = listOf("audio"),
                ),
            ),
        )
        advanceUntilIdle()

        assertThat(harness.hostCoreController.platformOperationFailedCalls).containsExactly(
            RecordedFailure("op-crash", failureMessage, false),
            RecordedFailure(
                "op-after-crash",
                "Android capability requests must be resolved before host creation",
                true,
            ),
        ).inOrder()
    }
}
