package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.rust.HostTransportController
import com.ekkus.silentdisco.core.uniffi.FfiCoreNotification
import com.ekkus.silentdisco.core.uniffi.FfiDeliveryReport
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportDelivery
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportHandle
import com.ekkus.silentdisco.core.uniffi.FfiTransportEffect
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
import org.mockito.Mockito

/**
 * Android counterpart of the desktop stale-approval guard. A reconnecting
 * device reuses its claimed ID under a new request generation, so an approval
 * for the retired request must fail visibly, must not touch the native handle,
 * and must not consume or admit the replacement's pending route.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class MainViewModelStaleApprovalTest {
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
        repeat(200) {
            advanceUntilIdle()
            if (condition()) return
            Thread.sleep(5)
        }
        check(condition()) { "Condition was not met after waiting for a real Dispatchers.IO hop" }
    }

    private fun approval(operationId: String, requestId: String) =
        FfiTransportEffect.DeliverJoinApproval(
            operationId = operationId,
            requestId = requestId,
            sessionId = "session-1",
            listenerId = "listener-1",
            trustedForFuture = false,
        )

    @Test
    fun staleRequestApprovalFailsWithoutTouchingNativeOrTheReplacementRoute() = runTest(dispatcher) {
        val native = Mockito.mock(FfiHostTransportHandle::class.java)
        Mockito.doAnswer { Thread.sleep(10); null }.`when`(native).pollEvent(500uL)
        Mockito.`when`(native.sendJoinApproval("listener-1", false))
            .thenReturn(FfiHostTransportDelivery(1u, 1u, 0u, 24uL))
        val transport = HostTransportController(
            callDispatcher = Dispatchers.IO,
            bindHandle = { _, _, _, _, _ -> native },
        )
        val harness = newTestMainViewModelHarness(hostTransportController = transport)
        try {
            transport.bind(backgroundScope, "127.0.0.1", 1, 2, 3, "session-1")
            harness.viewModel.ensureRustHostCore()
            advanceUntilIdle()
            // The retired generation's route is gone; the same claimed ID now
            // owns a different pending request.
            assertThat(
                harness.viewModel.pendingListenerDatagramPorts.record(
                    "req-2", "listener-1", 101u, 102u, native, controlAddress = "127.0.0.1:10002",
                ),
            ).isTrue()

            harness.hostCoreController.emit(
                FfiCoreNotification.TransportEffect(approval("op-stale", "req-1")),
            )
            awaitRealDispatchHop { harness.hostCoreController.transportDeliveryCompletedCalls.isNotEmpty() }

            assertThat(harness.hostCoreController.transportDeliveryCompletedCalls).containsExactly(
                "op-stale" to FfiDeliveryReport(intendedPeers = 1u, successfulPeers = 0u, failedPeers = 1u),
            )
            Mockito.verify(native, Mockito.never()).sendJoinApproval("listener-1", false)
            assertThat(harness.hostCoreController.submitListenerConnectedCalls).isEmpty()
            assertThat(harness.viewModel.pendingListenerDatagramPorts.size).isEqualTo(1)

            // Only the stale request is refused: the current request still
            // approves and authorizes exactly its own recorded ports.
            harness.hostCoreController.emit(
                FfiCoreNotification.TransportEffect(approval("op-current", "req-2")),
            )
            awaitRealDispatchHop { harness.hostCoreController.submitListenerConnectedCalls.isNotEmpty() }

            Mockito.verify(native).sendJoinApproval("listener-1", false)
            Mockito.verify(native).authorizeListener("listener-1", 101u, 102u)
            assertThat(harness.hostCoreController.submitListenerConnectedCalls.single().deviceId)
                .isEqualTo("listener-1")
            assertThat(harness.viewModel.pendingListenerDatagramPorts.size).isEqualTo(0)
        } finally {
            transport.close()
        }
    }
}
