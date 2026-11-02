package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Test

class ManualListenerLifetimeTest {
    @Test
    fun terminalCloseRejectsQueuedManualBindWithoutOpeningNativeSocket() {
        val lifetime = ManualListenerLifetime()
        val gate = ListenerNativeHandleGate<Any>()
        val original = Any()
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        val opens = AtomicInteger()
        val pool = Executors.newFixedThreadPool(2)
        try {
            gate.replace(retire = {}, open = { original }, start = {})
            val blockingSend = pool.submit {
                gate.withExpected(original) {
                    entered.countDown()
                    check(release.await(5, TimeUnit.SECONDS))
                }
            }
            assertThat(entered.await(5, TimeUnit.SECONDS)).isTrue()
            val queuedBind = pool.submit<Boolean> {
                gate.replaceIf(
                    shouldReplace = lifetime::isOpen,
                    retire = {},
                    open = { opens.incrementAndGet(); Any() },
                    start = {},
                )
            }
            assertThat(lifetime.closeOnce()).isTrue()
            release.countDown()
            blockingSend.get(5, TimeUnit.SECONDS)
            assertThat(queuedBind.get(5, TimeUnit.SECONDS)).isFalse()
            assertThat(opens.get()).isEqualTo(0)
            gate.close(retire = {})
            assertThat(gate.currentHandle()).isNull()
        } finally {
            release.countDown()
            pool.shutdownNow()
        }
    }

    @Test
    fun closeIsTerminalAndIdempotentAcrossRepeatedConnectAttempts() {
        val lifetime = ManualListenerLifetime()
        lifetime.ensureOpen()
        assertThat(lifetime.closeOnce()).isTrue()
        repeat(128) {
            assertThat(lifetime.isOpen()).isFalse()
            assertThat(lifetime.closeOnce()).isFalse()
            val failure = runCatching { lifetime.ensureOpen() }.exceptionOrNull()
            assertThat(failure).isInstanceOf(IllegalStateException::class.java)
        }
    }
}
