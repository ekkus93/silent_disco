package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import java.util.concurrent.CompletableFuture
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.flow.take
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import org.junit.Test

class BoundedCoreNotificationQueueTest {
    @Test
    fun slowCollectorBackpressuresCriticalEffectsWithoutReordering() {
        val queue = BoundedCoreNotificationQueue<String>(capacity = 1, criticalTimeoutMs = 2_000)
        try {
            queue.publishCritical("approve")
            val producer = CompletableFuture.runAsync { queue.publishCritical("stop") }
            Thread.sleep(40)
            assertThat(producer.isDone).isFalse()
            val delivered = runBlocking { queue.events.take(2).toList() }
            producer.get(2, TimeUnit.SECONDS)
            assertThat(delivered).containsExactly("approve", "stop").inOrder()
            assertThat(queue.criticalTimeouts).isEqualTo(0)
        } finally {
            queue.close()
        }
    }

    @Test
    fun diagnosticFloodCannotEvictQueuedAuthoritativeEffect() {
        val queue = BoundedCoreNotificationQueue<String>(capacity = 2)
        try {
            queue.publishCritical("join")
            queue.publishBestEffort("diagnostic")
            repeat(1_000) { queue.publishBestEffort("diagnostic-$it") }
            assertThat(queue.droppedDiagnostics).isEqualTo(1_000)
            val delivered = runBlocking { queue.events.take(2).toList() }
            assertThat(delivered).containsExactly("join", "diagnostic").inOrder()
        } finally {
            queue.close()
        }
    }

    @Test
    fun stoppedCollectorFailsCriticalDeliveryWithinFiniteDeadline() {
        val queue = BoundedCoreNotificationQueue<String>(capacity = 1, criticalTimeoutMs = 30)
        try {
            queue.publishCritical("first")
            val failure = runCatching { queue.publishCritical("second") }.exceptionOrNull()
            assertThat(failure).isInstanceOf(IllegalStateException::class.java)
            assertThat(failure?.message).contains("critical notification queue saturated")
            assertThat(queue.criticalTimeouts).isEqualTo(1)
            assertThat(runBlocking { queue.events.take(1).toList() }).containsExactly("first")
        } finally {
            queue.close()
        }
    }

    @Test
    fun closingQueueReleasesBlockedCriticalProducerWithoutSilentLoss() {
        val queue = BoundedCoreNotificationQueue<String>(capacity = 1, criticalTimeoutMs = 2_000)
        queue.publishCritical("join")
        val started = java.util.concurrent.CountDownLatch(1)
        val producer = CompletableFuture.supplyAsync {
            started.countDown()
            runCatching { queue.publishCritical("stop") }.exceptionOrNull()
        }
        assertThat(started.await(1, TimeUnit.SECONDS)).isTrue()
        Thread.sleep(40)
        assertThat(producer.isDone).isFalse()
        queue.close()
        val failure = producer.get(2, TimeUnit.SECONDS)
        assertThat(failure).isNotNull()
        assertThat(queue.criticalTimeouts).isEqualTo(0)
        assertThat(runBlocking { queue.events.toList() }).containsExactly("join")
    }
}
