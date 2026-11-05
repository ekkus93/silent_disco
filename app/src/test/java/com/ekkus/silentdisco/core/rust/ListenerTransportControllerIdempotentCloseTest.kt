package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportHandle
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertThrows
import org.junit.Test
import org.mockito.Mockito

/**
 * Listener teardown can be reached from Leave, a failed join, network loss,
 * profile switch and ViewModel clearing. Repeating any of them must send Leave
 * at most once and shut down and release each native handle exactly once.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class ListenerTransportControllerIdempotentCloseTest {
    private fun nativeHandle(): FfiListenerTransportHandle {
        val native = Mockito.mock(FfiListenerTransportHandle::class.java)
        Mockito.doAnswer { Thread.sleep(10); null }.`when`(native).pollEvent(500uL)
        return native
    }

    private fun controllerOpening(vararg handles: FfiListenerTransportHandle): ListenerTransportController {
        val queue = ArrayDeque(handles.toList())
        return ListenerTransportController(openHandle = { _, _, _, _ -> queue.removeFirst() })
    }

    @Test
    fun closeWithoutAnyConnectionIsANoOpAndBlocksLaterConnects() = runTest(StandardTestDispatcher()) {
        var opened = 0
        val controller = ListenerTransportController(
            openHandle = { _, _, _, _ -> opened++; nativeHandle() },
        )
        controller.close()
        controller.close()

        val refused = assertThrows(IllegalStateException::class.java) {
            kotlinx.coroutines.runBlocking {
                controller.connect(backgroundScope, "endpoint", "device", "127.0.0.1")
            }
        }
        assertThat(refused).hasMessageThat().contains("closed")
        assertThat(opened).isEqualTo(0)
    }

    @Test
    fun repeatedDisconnectSendsLeaveOnceAndReleasesTheHandleOnce() = runTest(StandardTestDispatcher()) {
        val native = nativeHandle()
        val controller = controllerOpening(native)
        controller.connect(backgroundScope, "endpoint", "device", "127.0.0.1")

        controller.disconnect("Leaving")
        controller.disconnect("Leaving")

        Mockito.verify(native, Mockito.times(1)).sendDisconnect("Leaving")
        Mockito.verify(native, Mockito.times(1)).shutdown()
        Mockito.verify(native, Mockito.times(1)).close()
        controller.close()
    }

    @Test
    fun repeatedCloseAfterConnectReleasesOnceWithoutSendingLeave() = runTest(StandardTestDispatcher()) {
        val native = nativeHandle()
        val controller = controllerOpening(native)
        controller.connect(backgroundScope, "endpoint", "device", "127.0.0.1")

        controller.close()
        controller.close()
        controller.disconnect("Late Leave")

        Mockito.verify(native, Mockito.never()).sendDisconnect(Mockito.anyString())
        Mockito.verify(native, Mockito.times(1)).shutdown()
        Mockito.verify(native, Mockito.times(1)).close()
    }

    @Test
    fun reconnectRetiresEachGenerationExactlyOnce() = runTest(StandardTestDispatcher()) {
        val first = nativeHandle()
        val second = nativeHandle()
        val controller = controllerOpening(first, second)
        controller.connect(backgroundScope, "endpoint", "device", "127.0.0.1")
        controller.connect(backgroundScope, "endpoint", "device", "127.0.0.1")
        controller.disconnect("Leaving")
        controller.close()
        controller.close()

        Mockito.verify(first, Mockito.times(1)).shutdown()
        Mockito.verify(first, Mockito.times(1)).close()
        Mockito.verify(second, Mockito.times(1)).sendDisconnect("Leaving")
        Mockito.verify(second, Mockito.times(1)).shutdown()
        Mockito.verify(second, Mockito.times(1)).close()
    }

    @Test
    fun repeatedReconnectsNeverCloseAPolledHandleOrLeaveAPollerOnARetiredOne() = runTest(StandardTestDispatcher()) {
        val violations = java.util.concurrent.CopyOnWriteArrayList<String>()
        val handles = List(8) { index ->
            val closed = java.util.concurrent.atomic.AtomicBoolean(false)
            val active = java.util.concurrent.atomic.AtomicInteger(0)
            val native = Mockito.mock(FfiListenerTransportHandle::class.java)
            Mockito.doAnswer {
                if (closed.get()) violations += "handle $index polled after close"
                if (active.incrementAndGet() > 1) violations += "handle $index has two concurrent pollers"
                try {
                    Thread.sleep(5)
                } finally {
                    active.decrementAndGet()
                }
                null
            }.`when`(native).pollEvent(500uL)
            Mockito.doAnswer {
                if (active.get() > 0) violations += "handle $index closed while a poll was in flight"
                closed.set(true)
                null
            }.`when`(native).close()
            native
        }
        val controller = controllerOpening(*handles.toTypedArray())
        handles.forEach { _ ->
            controller.connect(backgroundScope, "endpoint", "device", "127.0.0.1")
            Thread.sleep(20)
        }
        controller.close()
        // Give a leaked poller time to show itself.
        Thread.sleep(300)

        assertThat(violations).isEmpty()
        handles.forEach {
            Mockito.verify(it, Mockito.atLeastOnce()).pollEvent(500uL)
            Mockito.verify(it, Mockito.times(1)).close()
        }
    }
}
