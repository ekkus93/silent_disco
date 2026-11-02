package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Test

class HostTransportSessionGenerationTest {
    @Test
    fun stopInvalidatesQueuedBindWithoutRetiringOrReopeningWrongHandle() {
        val generations = HostTransportSessionGeneration()
        val gate = HostNativeHandleGate<Any>()
        val original = Any()
        val enteredSend = CountDownLatch(1)
        val releaseSend = CountDownLatch(1)
        val bindStarted = CountDownLatch(1)
        val opened = AtomicInteger()
        val retired = mutableListOf<Any>()
        val pool = Executors.newFixedThreadPool(2)
        try {
            gate.replace(retire = {}, open = { original }, start = {})
            val sending = pool.submit {
                gate.withExpected(original) {
                    enteredSend.countDown()
                    check(releaseSend.await(5, TimeUnit.SECONDS))
                }
            }
            assertThat(enteredSend.await(5, TimeUnit.SECONDS)).isTrue()

            val queuedGeneration = generations.beginBind()
            val queuedBind = pool.submit<Boolean> {
                bindStarted.countDown()
                gate.replaceIf(
                    shouldReplace = { generations.isCurrent(queuedGeneration) },
                    retire = { if (it != null) retired.add(it) },
                    open = { opened.incrementAndGet(); Any() },
                    start = {},
                )
            }
            assertThat(bindStarted.await(5, TimeUnit.SECONDS)).isTrue()
            // Stop invalidates the pending IO request even while a send holds
            // the native gate. Closing the handle cannot permit resurrection.
            generations.retire()
            releaseSend.countDown()
            sending.get(5, TimeUnit.SECONDS)
            assertThat(queuedBind.get(5, TimeUnit.SECONDS)).isFalse()
            assertThat(opened.get()).isEqualTo(0)
            assertThat(gate.currentHandle()).isSameInstanceAs(original)
            assertThat(retired).isEmpty()
            gate.close { if (it != null) retired.add(it) }
            assertThat(retired).containsExactly(original)

            // Close is not terminal: a deliberately new host session can bind.
            val nextGeneration = generations.beginBind()
            val replacement = Any()
            assertThat(gate.replaceIf(
                shouldReplace = { generations.isCurrent(nextGeneration) },
                retire = {},
                open = { replacement },
                start = {},
            )).isTrue()
            assertThat(gate.currentHandle()).isSameInstanceAs(replacement)
            gate.close {}
        } finally {
            releaseSend.countDown()
            pool.shutdownNow()
        }
    }

    @Test
    fun newestBindWinsAndOldRequestedSessionCannotReplaceIt() {
        val generations = HostTransportSessionGeneration()
        val gate = HostNativeHandleGate<Any>()
        val stale = generations.beginBind()
        val latest = generations.beginBind()
        val replacement = Any()
        val opened = AtomicInteger()
        assertThat(gate.replaceIf(
            shouldReplace = { generations.isCurrent(stale) },
            retire = {},
            open = { opened.incrementAndGet(); Any() },
            start = {},
        )).isFalse()
        assertThat(gate.replaceIf(
            shouldReplace = { generations.isCurrent(latest) },
            retire = {},
            open = { opened.incrementAndGet(); replacement },
            start = {},
        )).isTrue()
        assertThat(opened.get()).isEqualTo(1)
        assertThat(gate.currentHandle()).isSameInstanceAs(replacement)
        generations.retire()
        assertThat(generations.isCurrent(latest)).isFalse()
        gate.close {}
    }
}
