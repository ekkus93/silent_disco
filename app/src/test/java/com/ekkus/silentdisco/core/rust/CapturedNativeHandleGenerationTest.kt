package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import org.junit.Test

/** A synchronous listener callback must never act on a newly rebound socket. */
class CapturedNativeHandleGenerationTest {
    @Test
    fun queuedSyncOrPlaybackActionCannotMigrateToReplacementHandle() {
        val gate = ListenerNativeHandleGate<Any>()
        val original = Any()
        val replacement = Any()
        gate.replace(retire = {}, open = { original }, start = {})
        val replacementEntered = CountDownLatch(1)
        val releaseReplacement = CountDownLatch(1)
        val replacementThread = Thread {
            gate.replaceIf(
                shouldReplace = {
                    replacementEntered.countDown()
                    check(releaseReplacement.await(5, TimeUnit.SECONDS))
                    true
                },
                retire = {},
                open = { replacement },
                start = {},
            )
        }
        val observed = AtomicReference<Any?>()
        val queuedAction = Thread {
            observed.set(gate.withCapturedCurrent { it })
        }
        try {
            replacementThread.start()
            assertThat(replacementEntered.await(5, TimeUnit.SECONDS)).isTrue()
            // The replacement owns the monitor but has not yet retired the
            // original. The action captures that original before it waits.
            queuedAction.start()
            val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5)
            while (queuedAction.state != Thread.State.BLOCKED && System.nanoTime() < deadline) {
                Thread.sleep(1)
            }
            assertThat(queuedAction.state).isEqualTo(Thread.State.BLOCKED)
            releaseReplacement.countDown()
            replacementThread.join(5_000)
            queuedAction.join(5_000)
            assertThat(replacementThread.isAlive).isFalse()
            assertThat(queuedAction.isAlive).isFalse()
            assertThat(observed.get()).isNull()
            assertThat(gate.currentHandle()).isSameInstanceAs(replacement)
            assertThat(gate.withCapturedCurrent { it }).isSameInstanceAs(replacement)
        } finally {
            releaseReplacement.countDown()
            replacementThread.join(5_000)
            queuedAction.join(5_000)
            gate.close {}
        }
    }

    @Test
    fun noActiveSessionNeverInvokesCapturedAction() {
        val gate = ListenerNativeHandleGate<Any>()
        val delivered: Any? = gate.withCapturedCurrent { error("no listener is connected") }
        assertThat(delivered).isNull()
    }
}
