package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Test

/**
 * Exercise native command ownership, not just poll retirement or admission.
 * The fake send blocks inside the same gate used by production FFI sends.
 */
class HostNativeHandleBlockedSendTest {
    @Test
    fun blockedNativeSendFinishesOnOriginalHandleBeforeCloseAndRebind() {
        val gate = HostNativeHandleGate<Any>()
        val old = Any()
        val replacement = Any()
        val sendEntered = CountDownLatch(1)
        val releaseSend = CountDownLatch(1)
        val rebindRequested = CountDownLatch(1)
        val retired = AtomicInteger()
        val pool = Executors.newFixedThreadPool(2)
        try {
            gate.replace(retire = {}, open = { old }, start = {})
            val send = pool.submit<String?> {
                gate.withExpected(old) { handle ->
                    assertThat(handle).isSameInstanceAs(old)
                    sendEntered.countDown()
                    check(releaseSend.await(5, TimeUnit.SECONDS))
                    // Rebind cannot retire or replace the handle mid-FFI call.
                    assertThat(retired.get()).isEqualTo(0)
                    assertThat(gate.currentHandle()).isSameInstanceAs(old)
                    "delivered-on-original"
                }
            }
            assertThat(sendEntered.await(5, TimeUnit.SECONDS)).isTrue()
            val rebind = pool.submit {
                rebindRequested.countDown()
                gate.replace(
                    retire = { if (it != null) retired.incrementAndGet() },
                    open = { replacement },
                    start = {},
                )
            }
            assertThat(rebindRequested.await(5, TimeUnit.SECONDS)).isTrue()
            releaseSend.countDown()
            assertThat(send.get(5, TimeUnit.SECONDS)).isEqualTo("delivered-on-original")
            rebind.get(5, TimeUnit.SECONDS)
            assertThat(retired.get()).isEqualTo(1)
            assertThat(gate.currentHandle()).isSameInstanceAs(replacement)
            assertThat(gate.withExpected(old) { "unexpected stale send reached replacement" }).isNull()
            assertThat(gate.withExpected(replacement) { "fresh" }).isEqualTo("fresh")
            gate.close { if (it != null) retired.incrementAndGet() }
            assertThat(retired.get()).isEqualTo(2)
        } finally {
            releaseSend.countDown()
            pool.shutdownNow()
        }
    }

    @Test
    fun repeatedBlockedSendsCannotCrossGenerationOrDoubleRetire() {
        val gate = HostNativeHandleGate<Any>()
        val handles = (0..128).map { Any() }
        val retired = AtomicInteger()
        val completedSends = AtomicInteger()
        val pool = Executors.newFixedThreadPool(2)
        try {
            gate.replace(retire = {}, open = { handles.first() }, start = {})
            repeat(128) { index ->
                val old = handles[index]
                val next = handles[index + 1]
                val entered = CountDownLatch(1)
                val release = CountDownLatch(1)
                try {
                    val send = pool.submit {
                        gate.withExpected(old) {
                            entered.countDown()
                            check(release.await(5, TimeUnit.SECONDS))
                            assertThat(it).isSameInstanceAs(old)
                            assertThat(retired.get()).isEqualTo(index)
                            completedSends.incrementAndGet()
                        }
                    }
                    assertThat(entered.await(5, TimeUnit.SECONDS)).isTrue()
                    val rebind = pool.submit {
                        gate.replace(
                            retire = { if (it != null) retired.incrementAndGet() },
                            open = { next },
                            start = {},
                        )
                    }
                    release.countDown()
                    send.get(5, TimeUnit.SECONDS)
                    rebind.get(5, TimeUnit.SECONDS)
                    assertThat(gate.withExpected(old) {
                        "unexpected delayed old-generation send"
                    }).isNull()
                    assertThat(gate.currentHandle()).isSameInstanceAs(next)
                    assertThat(retired.get()).isEqualTo(index + 1)
                } finally {
                    release.countDown()
                }
            }
            assertThat(completedSends.get()).isEqualTo(128)
            gate.close { if (it != null) retired.incrementAndGet() }
            gate.close { if (it != null) retired.incrementAndGet() }
            assertThat(retired.get()).isEqualTo(129)
            assertThat(gate.currentHandle()).isNull()
        } finally {
            pool.shutdownNow()
        }
    }

    @Test
    fun failedNativeSendReleasesOwnershipForCloseAndNewSession() {
        val gate = HostNativeHandleGate<Any>()
        val old = Any()
        val next = Any()
        val retired = mutableListOf<Any>()
        gate.replace(retire = {}, open = { old }, start = {})
        val failure = runCatching {
            gate.withExpected(old) { throw IllegalStateException("native send failed") }
        }
        assertThat(failure.exceptionOrNull()).hasMessageThat().contains("native send failed")
        gate.replace(
            retire = { if (it != null) retired.add(it) },
            open = { next },
            start = {},
        )
        assertThat(retired).containsExactly(old)
        assertThat(gate.withExpected(old) { "unexpected stale send" }).isNull()
        assertThat(gate.withExpected(next) { "new-session-send" }).isEqualTo("new-session-send")
        gate.close { if (it != null) retired.add(it) }
        assertThat(retired).containsExactly(old, next).inOrder()
    }
}
