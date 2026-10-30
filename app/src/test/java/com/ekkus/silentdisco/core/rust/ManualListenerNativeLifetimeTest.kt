package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import kotlin.concurrent.thread
import org.junit.Test

/** Manual and discovered listeners share the native lifetime gate. */
class ManualListenerNativeLifetimeTest {
    @Test
    fun blockedManualPollKeepsNativeObjectLiveUntilLeaseDrains() {
        val gate = ListenerNativeHandleGate<Any>()
        val old = Any()
        val replacement = Any()
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        val closes = AtomicInteger()
        gate.replace(retire = {}, open = { old }, start = {})
        val poll = thread {
            gate.withExpectedPoll(old) {
                entered.countDown()
                check(release.await(5, TimeUnit.SECONDS))
            }
        }
        assertThat(entered.await(5, TimeUnit.SECONDS)).isTrue()
        val retired = CountDownLatch(1)
        val closing = thread {
            gate.close { handle ->
                assertThat(handle).isSameInstanceAs(old)
                gate.deferCloseUntilPollDrained(old, { closes.incrementAndGet() }) {
                    throw AssertionError(it)
                }
                retired.countDown()
            }
        }
        assertThat(retired.await(5, TimeUnit.SECONDS)).isTrue()
        assertThat(gate.currentHandle()).isNull()
        assertThat(closes.get()).isEqualTo(0)
        gate.replace(retire = {}, open = { replacement }, start = {})
        assertThat(gate.withExpected(old) { "stale-send" }).isNull()
        assertThat(gate.withExpectedPoll(old) { "stale-event" }).isNull()
        assertThat(gate.withExpected(replacement) { "current-send" }).isEqualTo("current-send")
        release.countDown()
        poll.join(5_000)
        closing.join(5_000)
        assertThat(poll.isAlive).isFalse()
        assertThat(closes.get()).isEqualTo(1)
    }

    @Test
    fun repeatedManualRebindNeverRoutesStaleSendToReplacement() {
        val gate = ListenerNativeHandleGate<Any>()
        var previous: Any? = null
        val closes = AtomicInteger()
        repeat(128) {
            val next = Any()
            gate.replace(
                retire = { if (it != null) closes.incrementAndGet() },
                open = { next },
                start = {},
            )
            previous?.let { old ->
                assertThat(gate.withExpected(old) { "stale" }).isNull()
            }
            assertThat(gate.withExpected(next) { "current" }).isEqualTo("current")
            previous = next
        }
        gate.close { if (it != null) closes.incrementAndGet() }
        assertThat(closes.get()).isEqualTo(128)
        assertThat(gate.currentHandle()).isNull()
    }
}
