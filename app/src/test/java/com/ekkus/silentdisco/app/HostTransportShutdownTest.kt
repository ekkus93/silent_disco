package com.ekkus.silentdisco.app

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
class HostTransportShutdownTest {
    @Test
    fun nativeRetirementSuspendsCallerWithoutBlockingItsThread() = runBlocking {
        val callingThread = Thread.currentThread()
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        val closedOn = AtomicReference<Thread>()
        val transport = AutoCloseable {
            closedOn.set(Thread.currentThread())
            entered.countDown()
            check(release.await(5, TimeUnit.SECONDS)) { "release native shutdown" }
        }
        val retiring = async(start = CoroutineStart.UNDISPATCHED) {
            retireHostTransportOffMain(transport)
        }
        try {
            assertThat(entered.await(5, TimeUnit.SECONDS)).isTrue()
            assertThat(closedOn.get()).isNotSameInstanceAs(callingThread)
            assertThat(retiring.isCompleted).isFalse()
        } finally {
            release.countDown()
        }
        retiring.await()
    }

    @Test
    fun cancellationCannotSkipAlreadyRequestedNativeRetirement() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        val calls = AtomicInteger()
        val retiring = backgroundScope.launch(start = CoroutineStart.UNDISPATCHED) {
            retireHostTransportOffMain(AutoCloseable { calls.incrementAndGet() }, dispatcher)
        }
        // The close is queued on IO, but the caller is cancelled first.
        retiring.cancel()
        runCurrent()
        assertThat(calls.get()).isEqualTo(1)
    }

    @Test
    fun nativeShutdownFailureIsPropagatedToHostEffectRecovery() = runBlocking {
        val failure = IllegalStateException("native shutdown failed")
        val actual = runCatching {
            retireHostTransportOffMain(AutoCloseable { throw failure })
        }.exceptionOrNull()
        assertThat(actual).isSameInstanceAs(failure)
    }
}
