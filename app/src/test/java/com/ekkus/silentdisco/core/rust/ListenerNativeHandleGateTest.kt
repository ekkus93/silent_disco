package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Test

/** Listener sends, playback attachment and teardown share one native lifetime gate. */
class ListenerNativeHandleGateTest {
    @Test
    fun blockedSendPreventsDisconnectFromClosingLiveNativeHandle() {
        val gate = ListenerNativeHandleGate<Any>()
        val old = Any()
        val started = CountDownLatch(1)
        val release = CountDownLatch(1)
        val closeStarted = CountDownLatch(1)
        val closed = AtomicInteger()
        val pool = Executors.newFixedThreadPool(2)
        try {
            gate.replace(retire = {}, open = { old }, start = {})
            val send = pool.submit {
                gate.withExpected(old) {
                    started.countDown()
                    check(release.await(5, TimeUnit.SECONDS))
                    assertThat(closed.get()).isEqualTo(0)
                }
            }
            assertThat(started.await(5, TimeUnit.SECONDS)).isTrue()
            val disconnect = pool.submit {
                closeStarted.countDown()
                gate.close { if (it != null) closed.incrementAndGet() }
            }
            assertThat(closeStarted.await(5, TimeUnit.SECONDS)).isTrue()
            assertThat(closed.get()).isEqualTo(0)
            release.countDown()
            send.get(5, TimeUnit.SECONDS)
            disconnect.get(5, TimeUnit.SECONDS)
            assertThat(closed.get()).isEqualTo(1)
            assertThat(gate.currentHandle()).isNull()
        } finally {
            release.countDown()
            pool.shutdownNow()
        }
    }

    @Test
    fun queuedCommandsAndRetiredPollEventsCannotCrossListenerRebind() {
        val gate = ListenerNativeHandleGate<Any>()
        val retired = Any()
        val replacement = Any()
        val closed = mutableListOf<Any>()
        gate.replace(retire = {}, open = { retired }, start = {})
        val capturedBeforeDispatch = gate.currentHandle()
        gate.replace(
            retire = { if (it != null) closed.add(it) },
            open = { replacement },
            start = {},
        )
        assertThat(gate.withExpected(capturedBeforeDispatch!!) { "stale join" }).isNull()
        assertThat(gate.withExpected(replacement) { "new join" }).isEqualTo("new join")
        assertThat(shouldForwardPolledListenerEvent(
            loopActive = true,
            handleIsCurrent = gate.currentHandle() === retired,
        )).isFalse()
        assertThat(closed).containsExactly(retired)
        gate.close { if (it != null) closed.add(it) }
        assertThat(closed).containsExactly(retired, replacement).inOrder()
    }

    @Test
    fun repeatedReconnectAndCleanupNeverRetainOrRecloseOldGenerations() {
        val gate = ListenerNativeHandleGate<Any>()
        val retired = mutableListOf<Any>()
        val handles = (0 until 128).map { Any() }
        val retire: (Any?) -> Unit = { if (it != null) retired.add(it) }
        handles.forEach { handle ->
            val expectedBeforeRebind = gate.currentHandle()
            gate.replace(retire = retire, open = { handle }, start = {})
            if (expectedBeforeRebind != null) {
                assertThat(gate.withExpected(expectedBeforeRebind) { "stale" }).isNull()
            }
            assertThat(gate.withExpected(handle) { "current" }).isEqualTo("current")
        }
        gate.close(retire)
        gate.close(retire)
        assertThat(retired).containsExactlyElementsIn(handles).inOrder()
        assertThat(gate.currentHandle()).isNull()
    }
}
