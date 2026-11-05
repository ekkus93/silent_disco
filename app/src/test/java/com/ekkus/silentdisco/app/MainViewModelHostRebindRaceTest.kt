package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.permissions.PermissionCatalogue
import com.ekkus.silentdisco.core.uniffi.FfiApprovalMode
import com.ekkus.silentdisco.core.rust.HostTransportController
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.transport.TransportSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiCoreNotification
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportHandle
import com.ekkus.silentdisco.core.uniffi.FfiPlatformCompletion
import com.ekkus.silentdisco.core.uniffi.FfiPlatformEffect
import com.google.common.truth.Truth.assertThat
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
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
import org.mockito.Mockito

/**
 * ViewModel-level host session races: Stop arriving while a native bind is in
 * flight, and repeated session switches. The real `HostTransportController`
 * and its generation gate are used; only the native handle is a mock.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class MainViewModelHostRebindRaceTest {
    private val dispatcher = StandardTestDispatcher()

    @Before
    fun setUpMain() {
        Dispatchers.setMain(dispatcher)
    }

    @After
    fun resetMain() {
        Dispatchers.resetMain()
    }

    private fun TestScope.awaitRealDispatchHop(condition: () -> Boolean) {
        repeat(400) {
            advanceUntilIdle()
            if (condition()) return
            Thread.sleep(5)
        }
        check(condition()) { "Condition was not met after waiting for a real Dispatchers.IO hop" }
    }

    private fun nativeHandle(): FfiHostTransportHandle {
        val native = Mockito.mock(FfiHostTransportHandle::class.java)
        Mockito.doAnswer { Thread.sleep(10); null }.`when`(native).pollEvent(500uL)
        return native
    }

    /** Production path: the start effect arms the bind, then Wi-Fi Direct reports ADVERTISING. */
    private fun TestScope.startHosting(harness: MainViewModelHarness, sessionId: String, operationId: String) {
        (PermissionCatalogue.wifiDirectPermissions() + PermissionCatalogue.bluetoothPermissions()).forEach {
            harness.viewModel.updatePermission(it.androidPermission, true)
        }
        harness.hostCoreController.emit(
            FfiCoreNotification.PlatformEffect(
                FfiPlatformEffect.StartAdvertising(
                    operationId = operationId,
                    sessionId = sessionId,
                    hostDeviceId = "host-1",
                    sessionName = "Patio Mix",
                    approvalMode = FfiApprovalMode.MANUAL,
                ),
            ),
        )
        advanceUntilIdle()
        // A StateFlow drops equal values, so a new session needs a fresh transition.
        harness.wifiDirectService.emitSnapshot(TransportSnapshot())
        advanceUntilIdle()
        harness.wifiDirectService.emitSnapshot(advertising)
        advanceUntilIdle()
    }

    private val advertising = TransportSnapshot(
        state = TransportConnectionState.ADVERTISING,
        hostAddressHint = "127.0.0.1",
    )

    @Test
    fun stopDuringInFlightBindRetiresTheNewHandleAndNeverAcknowledgesTheRetiredStart() =
        runTest(dispatcher) {
            val native = nativeHandle()
            val opening = CountDownLatch(1)
            val release = CountDownLatch(1)
            val transport = HostTransportController(
                callDispatcher = Dispatchers.IO,
                bindHandle = { _, _, _, _, _ ->
                    opening.countDown()
                    check(release.await(10, TimeUnit.SECONDS)) { "bind was never released" }
                    native
                },
            )
            val harness = newTestMainViewModelHarness(hostTransportController = transport)
            try {
                harness.viewModel.ensureRustHostCore()
                advanceUntilIdle()
                startHosting(harness, "session-1", "op-start")
                assertThat(opening.await(10, TimeUnit.SECONDS)).isTrue()

                // Stop arrives while the native bind is blocked.
                harness.hostCoreController.emit(
                    FfiCoreNotification.PlatformEffect(FfiPlatformEffect.StopAdvertising("op-stop")),
                )
                awaitRealDispatchHop { harness.viewModel.pendingStartAdvertisingOperationId == null }
                release.countDown()
                awaitRealDispatchHop { harness.hostCoreController.platformOperationSucceededCalls.isNotEmpty() }

                // The retired start is never acknowledged; only Stop is.
                assertThat(harness.hostCoreController.platformOperationSucceededCalls.map { it.operationId })
                    .containsExactly("op-stop")
                assertThat(harness.hostCoreController.platformOperationFailedCalls).isEmpty()
                // Whatever handle the in-flight bind produced is retired once.
                Mockito.verify(native, Mockito.times(1)).shutdown()
                Mockito.verify(native, Mockito.times(1)).close()
            } finally {
                release.countDown()
                transport.close()
            }
        }

    @Test
    fun repeatedSessionSwitchesRetireEachNativeGenerationExactlyOnce() = runTest(dispatcher) {
        val handles = List(CYCLES) { nativeHandle() }
        val queue = ArrayDeque(handles)
        val transport = HostTransportController(
            callDispatcher = Dispatchers.IO,
            bindHandle = { _, _, _, _, _ -> queue.removeFirst() },
        )
        val harness = newTestMainViewModelHarness(hostTransportController = transport)
        try {
            harness.viewModel.ensureRustHostCore()
            advanceUntilIdle()
            repeat(CYCLES) { cycle ->
                startHosting(harness, "session-$cycle", "op-$cycle")
                awaitRealDispatchHop {
                    harness.hostCoreController.platformOperationSucceededCalls.size == cycle + 1
                }
                // Switching sessions retires the previous generation, not this one.
                if (cycle > 0) {
                    Mockito.verify(handles[cycle - 1], Mockito.times(1)).close()
                }
                Mockito.verify(handles[cycle], Mockito.never()).close()
            }
            harness.hostCoreController.emit(
                FfiCoreNotification.PlatformEffect(FfiPlatformEffect.StopAdvertising("op-final")),
            )
            awaitRealDispatchHop {
                harness.hostCoreController.platformOperationSucceededCalls.size == CYCLES + 1
            }

            assertThat(harness.hostCoreController.platformOperationSucceededCalls.take(CYCLES).map { it.operationId })
                .containsExactlyElementsIn(List(CYCLES) { "op-$it" }).inOrder()
            assertThat(harness.hostCoreController.platformOperationSucceededCalls.last().completion)
                .isEqualTo(FfiPlatformCompletion.AdvertisingStopped)
            handles.forEach {
                Mockito.verify(it, Mockito.times(1)).shutdown()
                Mockito.verify(it, Mockito.times(1)).close()
            }
        } finally {
            transport.close()
        }
    }

    private companion object {
        const val CYCLES = 8
    }
}
