package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiHostTransportHandle
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Test
import org.mockito.Mockito

/**
 * Teardown must be safe to repeat: Stop, profile switch, ViewModel clearing and
 * a failed bind can all reach `close()`. Each native handle is shut down and
 * released exactly once, never twice, and never when nothing was bound.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class HostTransportControllerIdempotentCloseTest {
    private fun nativeHandle(): FfiHostTransportHandle {
        val native = Mockito.mock(FfiHostTransportHandle::class.java)
        Mockito.doAnswer { Thread.sleep(10); null }.`when`(native).pollEvent(500uL)
        return native
    }

    @Test
    fun closeWithoutAnyBoundSessionIsANoOp() {
        val controller = HostTransportController(callDispatcher = Dispatchers.IO)
        controller.close()
        controller.close()
    }

    @Test
    fun repeatedCloseShutsDownAndReleasesTheHandleExactlyOnce() = runTest(StandardTestDispatcher()) {
        val native = nativeHandle()
        val controller = HostTransportController(
            callDispatcher = Dispatchers.IO,
            bindHandle = { _, _, _, _, _ -> native },
        )
        controller.bind(backgroundScope, "127.0.0.1", 1, 2, 3, "session-1")

        controller.close()
        controller.close()
        controller.close()

        Mockito.verify(native, Mockito.times(1)).shutdown()
        Mockito.verify(native, Mockito.times(1)).close()
    }

    @Test
    fun rebindAfterCloseOpensAFreshHandleAndRetiresEachGenerationOnce() = runTest(StandardTestDispatcher()) {
        val first = nativeHandle()
        val second = nativeHandle()
        val handles = ArrayDeque(listOf(first, second))
        val controller = HostTransportController(
            callDispatcher = Dispatchers.IO,
            bindHandle = { _, _, _, _, _ -> handles.removeFirst() },
        )
        controller.bind(backgroundScope, "127.0.0.1", 1, 2, 3, "session-1")
        controller.close()
        controller.close()
        controller.bind(backgroundScope, "127.0.0.1", 1, 2, 3, "session-2")
        assertThat(handles).isEmpty()
        controller.close()
        controller.close()

        Mockito.verify(first, Mockito.times(1)).shutdown()
        Mockito.verify(first, Mockito.times(1)).close()
        Mockito.verify(second, Mockito.times(1)).shutdown()
        Mockito.verify(second, Mockito.times(1)).close()
    }
}
