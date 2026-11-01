package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Test

/** A superseded Wi-Fi advertising callback must not open or replace native sockets. */
class HostNativeHandleConditionalBindTest {
    @Test
    fun cancelledBindQueuedBehindNativeSendDoesNotReplaceAuthorizedSession() {
        val gate = HostNativeHandleGate<Any>()
        val live = Any()
        val enteredSend = CountDownLatch(1)
        val releaseSend = CountDownLatch(1)
        val requested = AtomicBoolean(true)
        val opens = AtomicInteger()
        val retired = AtomicInteger()
        val pool = Executors.newFixedThreadPool(2)
        try {
            gate.replace(retire = {}, open = { live }, start = {})
            val sending = pool.submit {
                gate.withExpected(live) {
                    enteredSend.countDown()
                    check(releaseSend.await(5, TimeUnit.SECONDS))
                }
            }
            assertThat(enteredSend.await(5, TimeUnit.SECONDS)).isTrue()
            val staleBind = pool.submit<Boolean> {
                gate.replaceIf(
                    shouldReplace = { requested.get() },
                    retire = { if (it != null) retired.incrementAndGet() },
                    open = { opens.incrementAndGet(); Any() },
                    start = {},
                )
            }
            requested.set(false) // Stop invalidates the old advertising operation.
            releaseSend.countDown()
            sending.get(5, TimeUnit.SECONDS)
            assertThat(staleBind.get(5, TimeUnit.SECONDS)).isFalse()
            assertThat(opens.get()).isEqualTo(0)
            assertThat(retired.get()).isEqualTo(0)
            assertThat(gate.currentHandle()).isSameInstanceAs(live)
            gate.close { if (it != null) retired.incrementAndGet() }
            assertThat(retired.get()).isEqualTo(1)
        } finally {
            releaseSend.countDown()
            pool.shutdownNow()
        }
    }

    @Test
    fun stoppedSessionCannotBeResurrectedByRepeatedDelayedBindCallbacks() {
        val gate = HostNativeHandleGate<Any>()
        val live = Any()
        val retired = AtomicInteger()
        val opens = AtomicInteger()
        gate.replace(retire = {}, open = { live }, start = {})
        gate.close { if (it != null) retired.incrementAndGet() }
        repeat(128) {
            assertThat(gate.replaceIf(
                shouldReplace = { false },
                retire = { if (it != null) retired.incrementAndGet() },
                open = { opens.incrementAndGet(); Any() },
                start = {},
            )).isFalse()
        }
        assertThat(gate.currentHandle()).isNull()
        assertThat(opens.get()).isEqualTo(0)
        assertThat(retired.get()).isEqualTo(1)
    }

    @Test
    fun currentBindStartsOnceAndSupersededCallbackPreservesReplacement() {
        val gate = HostNativeHandleGate<Any>()
        val first = Any()
        val next = Any()
        val retired = mutableListOf<Any>()
        val opened = AtomicInteger()
        assertThat(gate.replaceIf(
            shouldReplace = { true },
            retire = { if (it != null) retired.add(it) },
            open = { opened.incrementAndGet(); first },
            start = { assertThat(it).isSameInstanceAs(first) },
        )).isTrue()
        assertThat(gate.replaceIf(
            shouldReplace = { true },
            retire = { if (it != null) retired.add(it) },
            open = { opened.incrementAndGet(); next },
            start = {},
        )).isTrue()
        assertThat(gate.replaceIf(
            shouldReplace = { false },
            retire = { if (it != null) retired.add(it) },
            open = { error("retired operation must never open a socket") },
            start = {},
        )).isFalse()
        assertThat(gate.currentHandle()).isSameInstanceAs(next)
        assertThat(retired).containsExactly(first)
        assertThat(opened.get()).isEqualTo(2)
    }
}
