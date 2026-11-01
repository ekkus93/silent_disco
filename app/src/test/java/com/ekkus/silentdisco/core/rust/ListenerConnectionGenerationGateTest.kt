package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong
import org.junit.Test

/** Listener Leave and reconnect must be ordered by request generation, not IO scheduling. */
class ListenerConnectionGenerationGateTest {
    @Test
    fun queuedOldBindCannotResurrectConnectionAfterLeave() {
        val gate = HostNativeHandleGate<Any>()
        val generation = AtomicLong(1)
        val live = Any()
        val enteredSend = CountDownLatch(1)
        val releaseSend = CountDownLatch(1)
        val opened = AtomicInteger()
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
            val oldBind = pool.submit<Boolean> {
                gate.replaceIf(
                    shouldReplace = { generation.get() == 1L },
                    retire = {},
                    open = { opened.incrementAndGet(); Any() },
                    start = {},
                )
            }
            val leave = generation.incrementAndGet()
            releaseSend.countDown()
            sending.get(5, TimeUnit.SECONDS)
            assertThat(oldBind.get(5, TimeUnit.SECONDS)).isFalse()
            assertThat(opened.get()).isEqualTo(0)
            assertThat(gate.closeIf({ generation.get() == leave }, retire = {})).isTrue()
            assertThat(gate.currentHandle()).isNull()
        } finally {
            releaseSend.countDown()
            pool.shutdownNow()
        }
    }

    @Test
    fun delayedLeaveCannotCloseReplacementConnection() {
        val gate = HostNativeHandleGate<Any>()
        val generation = AtomicLong(1)
        val original = Any()
        val replacement = Any()
        val retired = mutableListOf<Any>()
        gate.replace(retire = {}, open = { original }, start = {})
        val leaving = generation.incrementAndGet()
        val joining = generation.incrementAndGet()
        assertThat(gate.replaceIf(
            shouldReplace = { generation.get() == joining },
            retire = { if (it != null) retired.add(it) },
            open = { replacement },
            start = {},
        )).isTrue()
        assertThat(gate.closeIf(
            shouldClose = { generation.get() == leaving },
            retire = { if (it != null) retired.add(it) },
        )).isFalse()
        assertThat(gate.currentHandle()).isSameInstanceAs(replacement)
        assertThat(retired).containsExactly(original)
    }

    @Test
    fun disconnectDuringNativeBindClosesThatObsoleteGeneration() {
        val gate = HostNativeHandleGate<Any>()
        val generation = AtomicLong(1)
        val enteredOpen = CountDownLatch(1)
        val releaseOpen = CountDownLatch(1)
        val retired = AtomicInteger()
        val pool = Executors.newFixedThreadPool(2)
        try {
            val binding = pool.submit<Boolean> {
                gate.replaceIf(
                    shouldReplace = { generation.get() == 1L },
                    retire = {},
                    open = {
                        enteredOpen.countDown()
                        check(releaseOpen.await(5, TimeUnit.SECONDS))
                        Any()
                    },
                    start = {},
                )
            }
            assertThat(enteredOpen.await(5, TimeUnit.SECONDS)).isTrue()
            val leaving = generation.incrementAndGet()
            val closing = pool.submit<Boolean> {
                gate.closeIf(
                    shouldClose = { generation.get() == leaving },
                    retire = { if (it != null) retired.incrementAndGet() },
                )
            }
            releaseOpen.countDown()
            assertThat(binding.get(5, TimeUnit.SECONDS)).isTrue()
            assertThat(closing.get(5, TimeUnit.SECONDS)).isTrue()
            assertThat(retired.get()).isEqualTo(1)
            assertThat(gate.currentHandle()).isNull()
        } finally {
            releaseOpen.countDown()
            pool.shutdownNow()
        }
    }

    @Test
    fun terminalCloseRejectsRepeatedDelayedJoins() {
        val gate = HostNativeHandleGate<Any>()
        val generation = AtomicLong(1)
        val closed = AtomicBoolean(false)
        val opens = AtomicInteger()
        closed.set(true)
        generation.incrementAndGet()
        repeat(128) {
            assertThat(gate.replaceIf(
                shouldReplace = { !closed.get() && generation.get() == 1L },
                retire = {},
                open = { opens.incrementAndGet(); Any() },
                start = {},
            )).isFalse()
        }
        assertThat(opens.get()).isEqualTo(0)
        assertThat(gate.currentHandle()).isNull()
    }
}
