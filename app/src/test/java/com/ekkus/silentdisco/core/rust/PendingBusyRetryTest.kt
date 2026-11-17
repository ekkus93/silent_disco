package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.cancel
import kotlinx.coroutines.joinAll
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Test
import java.util.concurrent.atomic.AtomicInteger

@OptIn(ExperimentalCoroutinesApi::class)
class PendingBusyRetryTest {
    private fun TestScope.childScope() = CoroutineScope(SupervisorJob() + StandardTestDispatcher(testScheduler))

    @Test
    fun aRetryRunsOnceAfterItsDelayAndIsPendingOnlyWhileWaiting() = runTest {
        val scope = childScope()
        val retry = PendingBusyRetry()
        val events = mutableListOf<String>()

        retry.schedule(scope, delayMs = 2_000, beforeWait = { events += "retire" }) { events += "connect" }
        testScheduler.advanceTimeBy(1_999)
        testScheduler.runCurrent()
        assertThat(events).containsExactly("retire")
        assertThat(retry.isPending).isTrue()

        advanceUntilIdle()
        assertThat(events).containsExactly("retire", "connect").inOrder()
        assertThat(retry.isPending).isFalse()
        scope.cancel()
    }

    @Test
    fun cancellingDuringTheWaitMeansItNeverConnects() = runTest {
        val scope = childScope()
        val retry = PendingBusyRetry()
        var connects = 0

        retry.schedule(scope, delayMs = 4_000) { connects++ }
        testScheduler.advanceTimeBy(1_000)
        testScheduler.runCurrent()
        assertThat(retry.cancel()).isTrue()
        advanceUntilIdle()

        assertThat(connects).isEqualTo(0)
        assertThat(retry.isPending).isFalse()
        assertThat(retry.cancel()).isFalse()
        scope.cancel()
    }

    @Test
    fun cancellingWhileTheRefusedConnectionIsRetiringMeansItNeverConnects() = runTest {
        val scope = childScope()
        val retry = PendingBusyRetry()
        var connects = 0

        retry.schedule(scope, delayMs = 1_000, beforeWait = { awaitCancellation() }) { connects++ }
        testScheduler.runCurrent()
        assertThat(retry.cancel()).isTrue()
        advanceUntilIdle()

        assertThat(connects).isEqualTo(0)
        scope.cancel()
    }

    @Test
    fun aNewerRetryReplacesTheWaitingOne() = runTest {
        val scope = childScope()
        val retry = PendingBusyRetry()
        val connected = mutableListOf<Int>()

        retry.schedule(scope, delayMs = 4_000) { connected += 1 }
        testScheduler.advanceTimeBy(3_000)
        testScheduler.runCurrent()
        retry.schedule(scope, delayMs = 2_000) { connected += 2 }
        advanceUntilIdle()

        assertThat(connected).containsExactly(2)
        scope.cancel()
    }

    @Test
    fun aCancelAfterTheRetryStartedConnectingDoesNotInterruptIt() = runTest {
        val scope = childScope()
        val retry = PendingBusyRetry()
        val events = mutableListOf<String>()

        retry.schedule(scope, delayMs = 1_000) {
            events += "connect started"
            kotlinx.coroutines.delay(500)
            events += "connect finished"
        }
        testScheduler.advanceTimeBy(1_200)
        testScheduler.runCurrent()
        assertThat(retry.cancel()).isFalse()
        advanceUntilIdle()

        assertThat(events).containsExactly("connect started", "connect finished").inOrder()
        scope.cancel()
    }

    /**
     * Real threads: the retry's delay ends on a worker thread while the test
     * thread cancels. In every round exactly one side must win. Both winning
     * is a stale reconnect after a successful cancel; neither is a lost retry.
     */
    @Test
    fun aCancelRacingTheEndOfTheDelayHasExactlyOneWinner() {
        val rounds = 2_000
        repeat(rounds) { round ->
            val parent = SupervisorJob()
            val scope = CoroutineScope(parent + Dispatchers.Default)
            val retry = PendingBusyRetry()
            val connects = AtomicInteger()

            retry.schedule(scope, delayMs = 0) { connects.incrementAndGet() }
            repeat(round % 50) { Thread.onSpinWait() }
            val dropped = retry.cancel()
            runBlocking { parent.children.toList().joinAll() }

            val ran = connects.get()
            assertThat(ran).isAtMost(1)
            assertThat(dropped xor (ran == 1)).isTrue()
            assertThat(retry.isPending).isFalse()
            scope.cancel()
        }
    }
}
