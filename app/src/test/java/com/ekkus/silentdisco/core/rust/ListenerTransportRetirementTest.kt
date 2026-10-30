package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class ListenerTransportRetirementTest {
    @Test
    fun listenerRetirementDoesNotBlockCallingThread() = runBlocking {
        val caller = Thread.currentThread()
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        val retiredOn = AtomicReference<Thread>()
        val job = async(start = CoroutineStart.UNDISPATCHED) {
            retireListenerTransportOffMain {
                retiredOn.set(Thread.currentThread())
                entered.countDown()
                check(release.await(5, TimeUnit.SECONDS)) { "release listener retirement" }
            }
        }
        try {
            assertThat(entered.await(5, TimeUnit.SECONDS)).isTrue()
            assertThat(retiredOn.get()).isNotSameInstanceAs(caller)
            assertThat(job.isCompleted).isFalse()
        } finally {
            release.countDown()
        }
        job.await()
    }

    @Test
    fun cancelledListenerEffectStillRetiresItsNativeGeneration() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        val calls = AtomicInteger()
        val job = backgroundScope.launch(start = CoroutineStart.UNDISPATCHED) {
            retireListenerTransportOffMain(dispatcher) { calls.incrementAndGet() }
        }
        // The IO retirement has not run yet, but the initiating coroutine
        // is already cancelled. Native close must still run exactly once.
        job.cancel()
        runCurrent()
        assertThat(calls.get()).isEqualTo(1)
    }

    @Test
    fun listenerRetirementFailureIsNotSilentlyDiscarded() = runBlocking {
        val actual = runCatching {
            retireListenerTransportOffMain {
                throw IllegalStateException("listener native shutdown failed")
            }
        }.exceptionOrNull()
        assertThat(actual).isInstanceOf(IllegalStateException::class.java)
        assertThat(actual?.message).isEqualTo("listener native shutdown failed")
    }
}
