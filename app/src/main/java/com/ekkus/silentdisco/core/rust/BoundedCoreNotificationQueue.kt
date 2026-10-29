package com.ekkus.silentdisco.core.rust

import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.selects.select
import kotlinx.coroutines.withTimeoutOrNull

private data class CoreNotificationEntry<Event : Any>(
    val event: Event,
    val bestEffort: Boolean,
)

/**
 * Finite Rust-actor-to-Android bridge. Snapshots are separately conflated in
 * StateFlow; only disposable diagnostics may be dropped under pressure.
 * Diagnostics occupy at most a quarter of the queue, reserving slots for
 * critical operations even when no collector is running. Critical effects
 * use bounded backpressure on the Rust observer thread,
 * never on the native audio callback. A stalled collector fails explicitly
 * instead of silently losing a transport/approval/Stop operation.
 */
internal class BoundedCoreNotificationQueue<Event : Any>(
    capacity: Int = DEFAULT_CAPACITY,
    private val criticalTimeoutMs: Long = DEFAULT_CRITICAL_TIMEOUT_MS,
) : AutoCloseable {
    init {
        require(capacity > 0)
        require(criticalTimeoutMs > 0)
    }

    private val channel = Channel<CoreNotificationEntry<Event>>(capacity)
    private val closed = CompletableDeferred<Unit>()
    private val dropped = AtomicLong()
    private val timedOut = AtomicLong()
    private val bestEffortLimit = if (capacity == 1) 0 else (capacity / 4).coerceIn(1, capacity - 1)
    private val queuedBestEffort = AtomicInteger()

    val events: Flow<Event> = flow {
        for (entry in channel) {
            if (entry.bestEffort) queuedBestEffort.decrementAndGet()
            emit(entry.event)
        }
    }
    val droppedDiagnostics: Long get() = dropped.get()
    val criticalTimeouts: Long get() = timedOut.get()

    fun publishBestEffort(event: Event) {
        val pending = queuedBestEffort.incrementAndGet()
        if (pending > bestEffortLimit) {
            queuedBestEffort.decrementAndGet()
            dropped.incrementAndGet()
            return
        }
        if (channel.trySend(CoreNotificationEntry(event, bestEffort = true)).isFailure) {
            queuedBestEffort.decrementAndGet()
            dropped.incrementAndGet()
        }
    }

    fun publishCritical(event: Event) {
        val entry = CoreNotificationEntry(event, bestEffort = false)
        if (channel.trySend(entry).isSuccess) return
        val delivered = runBlocking {
            withTimeoutOrNull(criticalTimeoutMs) {
                select<Boolean> {
                    channel.onSend(entry) { true }
                    closed.onAwait { error("Rust host critical notification queue closed") }
                }
            }
        } == true
        if (!delivered) {
            timedOut.incrementAndGet()
            error("Rust host critical notification queue saturated for ${criticalTimeoutMs}ms")
        }
    }

    override fun close() {
        // Channel.close() alone does not release senders suspended behind a
        // full buffer. Wake them without discarding already-queued controls.
        channel.close()
        closed.complete(Unit)
    }

    private companion object {
        const val DEFAULT_CAPACITY = 64
        const val DEFAULT_CRITICAL_TIMEOUT_MS = 2_000L
    }
}
