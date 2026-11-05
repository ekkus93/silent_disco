package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.take
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.withTimeout
import org.junit.Test

/**
 * The bridge queues are bounded in slots; PCM notifications carry byte arrays,
 * so the memory bound is slots times payload size. These tests use 64 KiB
 * payloads and measure the bytes the queues actually retain while no collector
 * runs, instead of only counting events.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class BridgeQueueRetentionTest {
    private fun payload(tag: Int) = ByteArray(PAYLOAD_BYTES).also { it[0] = tag.toByte() }

    private fun ByteArray.tag() = this[0].toInt() and 0xFF

    @Test
    fun stalledCollectorFloodRetainsAtMostCapacityTimesPayloadBytes() = runTest {
        val handle = Any()
        val queue = BoundedTransportEventQueue<Any, ByteArray>({ handle }, capacity = CAPACITY)

        // Far more PCM than the queue may hold, with nobody collecting.
        repeat(OFFERED_PCM) { queue.tryEmit(handle, payload(TELEMETRY)) }
        assertThat(queue.droppedBestEffort).isEqualTo((OFFERED_PCM - TELEMETRY_SLOTS).toLong())
        // Authoritative events keep their reserved share, then wait: they apply
        // backpressure to the poll coroutine instead of growing the queue.
        repeat(CRITICAL_SLOTS) { queue.emit(handle, payload(CRITICAL)) }
        val waiting = backgroundScope.async { queue.emit(handle, payload(WAITING)) }
        runCurrent()
        assertThat(waiting.isCompleted).isFalse()

        val retained = backgroundScope.async { queue.events.take(CAPACITY).toList() }
        runCurrent()
        val events = retained.await()

        assertThat(events.sumOf { it.size }).isEqualTo(CAPACITY * PAYLOAD_BYTES)
        assertThat(events.count { it.tag() == TELEMETRY }).isEqualTo(TELEMETRY_SLOTS)
        assertThat(events.count { it.tag() == CRITICAL }).isEqualTo(CRITICAL_SLOTS)
        // The waiting authoritative event is delivered once room appears.
        val last = backgroundScope.async { queue.events.take(1).toList() }
        runCurrent()
        assertThat(last.await().single().tag()).isEqualTo(WAITING)
        queue.close()
    }

    @Test
    fun rapidRebindsRetainOnlyTheCurrentSessionsBytes() = runTest {
        var current = Any()
        val queue = BoundedTransportEventQueue<Any, ByteArray>({ current }, capacity = CAPACITY)

        repeat(SESSIONS) { session ->
            // The same retire-then-open order as a host rebind.
            current = Any()
            queue.discardRetiredEvents()
            val handle = current
            repeat(OFFERED_PCM / 10) { queue.tryEmit(handle, payload(session)) }
            queue.emit(handle, payload(session))
        }

        val expected = TELEMETRY_SLOTS + 1
        val retained = backgroundScope.async { queue.events.take(expected).toList() }
        runCurrent()
        val events = retained.await()

        assertThat(events.map { it.tag() }.toSet()).containsExactly(SESSIONS - 1)
        assertThat(events.sumOf { it.size }).isEqualTo(expected * PAYLOAD_BYTES)
        queue.close()
    }

    @Test
    fun coreDiagnosticFloodRetainsAtMostTheDiagnosticShareInBytes() {
        val queue = BoundedCoreNotificationQueue<ByteArray>(capacity = CAPACITY, criticalTimeoutMs = 2_000)

        repeat(OFFERED_PCM) { queue.publishBestEffort(payload(TELEMETRY)) }
        assertThat(queue.droppedDiagnostics).isEqualTo((OFFERED_PCM - TELEMETRY_SLOTS).toLong())
        // Reserved slots stay available for join, approval, Stop and errors.
        repeat(CRITICAL_SLOTS) { queue.publishCritical(payload(CRITICAL)) }

        val events = runBlocking { withTimeout(5_000) { queue.events.take(CAPACITY).toList() } }

        assertThat(events.sumOf { it.size }).isEqualTo(CAPACITY * PAYLOAD_BYTES)
        assertThat(events.count { it.tag() == TELEMETRY }).isEqualTo(TELEMETRY_SLOTS)
        assertThat(events.count { it.tag() == CRITICAL }).isEqualTo(CRITICAL_SLOTS)
        queue.close()
    }

    private companion object {
        const val PAYLOAD_BYTES = 64 * 1024
        const val CAPACITY = 64
        // A quarter of the slots may hold lossy telemetry; the rest are reserved.
        const val TELEMETRY_SLOTS = 16
        const val CRITICAL_SLOTS = 48
        const val OFFERED_PCM = 1_000
        const val SESSIONS = 200
        const val TELEMETRY = 1
        const val CRITICAL = 2
        const val WAITING = 3
    }
}
