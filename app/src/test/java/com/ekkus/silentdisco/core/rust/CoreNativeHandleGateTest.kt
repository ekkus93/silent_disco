package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Test

class CoreNativeHandleGateTest {
    @Test
    fun closeWaitsForBlockedActorCallbackAndCannotReleaseNativeMemoryEarly() {
        val opened = AtomicInteger()
        val retired = AtomicInteger()
        val gate = CoreNativeHandleGate(
            open = { opened.incrementAndGet(); Any() },
            retire = { retired.incrementAndGet() },
        )
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        val closing = CountDownLatch(1)
        val pool = Executors.newFixedThreadPool(2)
        try {
            gate.withOpen { }
            val callback = pool.submit {
                gate.withExisting {
                    entered.countDown()
                    check(release.await(5, TimeUnit.SECONDS))
                    assertThat(retired.get()).isEqualTo(0)
                }
            }
            assertThat(entered.await(5, TimeUnit.SECONDS)).isTrue()
            val shutdown = pool.submit {
                closing.countDown()
                gate.close()
            }
            assertThat(closing.await(5, TimeUnit.SECONDS)).isTrue()
            assertThat(retired.get()).isEqualTo(0)
            release.countDown()
            callback.get(5, TimeUnit.SECONDS)
            shutdown.get(5, TimeUnit.SECONDS)
            assertThat(opened.get()).isEqualTo(1)
            assertThat(retired.get()).isEqualTo(1)
            val staleCallback: String? = gate.withExisting { error("callback after close") }
            assertThat(staleCallback).isNull()
            assertThat(runCatching { gate.withOpen { } }.exceptionOrNull())
                .isInstanceOf(IllegalStateException::class.java)
        } finally {
            release.countDown()
            pool.shutdownNow()
        }
    }

    @Test
    fun closeAndFirstOpenAreSerializedWithoutResurrection() {
        val opened = AtomicInteger()
        val retired = AtomicInteger()
        val gate = CoreNativeHandleGate(
            open = { opened.incrementAndGet(); Any() },
            retire = { retired.incrementAndGet() },
        )
        val enteredOpen = CountDownLatch(1)
        val releaseOpen = CountDownLatch(1)
        val pool = Executors.newFixedThreadPool(2)
        try {
            val opening = pool.submit {
                gate.withOpen {
                    enteredOpen.countDown()
                    check(releaseOpen.await(5, TimeUnit.SECONDS))
                }
            }
            assertThat(enteredOpen.await(5, TimeUnit.SECONDS)).isTrue()
            val shutdown = pool.submit { gate.close() }
            releaseOpen.countDown()
            opening.get(5, TimeUnit.SECONDS)
            shutdown.get(5, TimeUnit.SECONDS)
            repeat(128) {
                val staleCallback: String? = gate.withExisting { error("stale actor callback") }
                assertThat(staleCallback).isNull()
                assertThat(runCatching { gate.withOpen { } }.isFailure).isTrue()
            }
            gate.close()
            assertThat(opened.get()).isEqualTo(1)
            assertThat(retired.get()).isEqualTo(1)
        } finally {
            releaseOpen.countDown()
            pool.shutdownNow()
        }
    }

    @Test
    fun shutdownFailureStillPermanentlyClosesGate() {
        val handle = Any()
        val gate = CoreNativeHandleGate(
            open = { handle },
            retire = { throw IllegalStateException("native shutdown failed") },
        )
        assertThat(gate.withOpen { it }).isSameInstanceAs(handle)
        assertThat(runCatching { gate.close() }.exceptionOrNull())
            .hasMessageThat().contains("native shutdown failed")
        val staleHandle: String? = gate.withExisting { error("stale handle") }
        assertThat(staleHandle).isNull()
        assertThat(runCatching { gate.withOpen { } }.isFailure).isTrue()
        gate.close()
    }

    @Test
    fun failedFirstSnapshotInitializationRetiresEveryNativeActorBeforeRetry() {
        val opened = AtomicInteger()
        val retired = mutableListOf<Int>()
        val gate = CoreNativeHandleGate(
            open = {
                openInitializedNativeHandle(
                    open = { opened.incrementAndGet() },
                    initialize = { if (it <= 128) error("initial snapshot unavailable") },
                    retire = { retired.add(it) },
                )
            },
            retire = { retired.add(it) },
        )
        repeat(128) {
            val failure = runCatching { gate.withOpen { it } }.exceptionOrNull()
            assertThat(failure).isInstanceOf(IllegalStateException::class.java)
            assertThat(failure?.message).contains("initial snapshot unavailable")
            // Failed open must not be cached as an apparently usable handle.
            val existing: Int? = gate.withExisting { it }
            assertThat(existing).isNull()
            assertThat(retired).hasSize(it + 1)
        }
        assertThat(gate.withOpen { it }).isEqualTo(129)
        gate.close()
        gate.close()
        assertThat(opened.get()).isEqualTo(129)
        assertThat(retired).containsExactlyElementsIn(1..129).inOrder()
    }

    @Test
    fun initializationFailurePreservesPrimaryErrorAndNativeTeardownFailures() {
        val primary = IllegalArgumentException("snapshot decode failed")
        val shutdown = IllegalStateException("shutdown failed")
        val close = IllegalStateException("close failed")
        val calls = mutableListOf<String>()
        val failure = runCatching {
            openInitializedNativeHandle(
                open = { "native-actor" },
                initialize = { throw primary },
                retire = {
                    retireNativeHandle(
                        it,
                        shutdown = { calls.add("shutdown"); throw shutdown },
                        close = { calls.add("close"); throw close },
                    )
                },
            )
        }.exceptionOrNull()
        assertThat(failure).isSameInstanceAs(primary)
        assertThat(calls).containsExactly("shutdown", "close").inOrder()
        assertThat(primary.suppressed).asList().containsExactly(shutdown)
        assertThat(shutdown.suppressed).asList().containsExactly(close)
    }

    @Test
    fun nativeAllocationFailureDoesNotAttemptToRetireUnallocatedHandle() {
        val allocated = IllegalStateException("native allocation failed")
        val retirementCount = AtomicInteger()
        val failure = runCatching {
            openInitializedNativeHandle(
                open = { throw allocated },
                initialize = { error("cannot initialize missing actor") },
                retire = { retirementCount.incrementAndGet() },
            )
        }.exceptionOrNull()
        assertThat(failure).isSameInstanceAs(allocated)
        assertThat(retirementCount.get()).isEqualTo(0)
    }

}
