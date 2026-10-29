package com.ekkus.silentdisco.core.rust

import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.selects.select

private data class TransportQueueEntry<H : Any, E : Any>(
    val handle: H,
    val event: E,
    val bestEffort: Boolean,
)

/**
 * Bounded native-poller-to-Android bridge. Critical events suspend only the
 * IO poll coroutine until the collector accepts them; they are never evicted
 * by a buffer overflow. High-rate telemetry/PCM uses explicit best-effort
 * admission and increments a diagnostic counter when full.
 *
 * Best-effort events may occupy at most one quarter of the queue, leaving
 * reserved slots for join/approval/Stop/disconnect even when the collector
 * is stalled and the audio receiver is producing at full rate. Critical
 * events retain their original FIFO order. The producer must not be a
 * real-time audio callback.
 *
 * Each item retains its native handle identity. A rebind can therefore never
 * deliver queued events from a retired generation to the next session.
 */
internal class BoundedTransportEventQueue<Handle : Any, Event : Any>(
    private val currentHandle: () -> Handle?,
    capacity: Int = DEFAULT_CAPACITY,
) : AutoCloseable {
    init {
        require(capacity > 0)
    }

    private val channel = Channel<TransportQueueEntry<Handle, Event>>(capacity)
    private val closed = CompletableDeferred<Unit>()
    private val bestEffortLimit = if (capacity == 1) 0 else (capacity / 4).coerceIn(1, capacity - 1)
    private val queuedBestEffort = AtomicInteger()
    private val dropped = AtomicLong()
    // Serializes lossy enqueues with rebind cleanup. A retired poller that
    // passed its outer generation check just before replacement must not
    // enqueue stale PCM *after* the replacement drains the old queue.
    private val enqueueGate = Any()

    // Decrement before emitting downstream, even when the generation is stale
    // or the collector is cancelled while processing a delivered event.
    val events: Flow<Event> = flow {
        for (queued in channel) {
            if (queued.bestEffort) queuedBestEffort.decrementAndGet()
            if (currentHandle() === queued.handle) emit(queued.event)
        }
    }

    val droppedBestEffort: Long get() = dropped.get()

    /**
     * Retire buffered events at a native session boundary, before the next
     * poller starts. Merely tagging events by handle is insufficient: if the
     * previous collector stalled with a full queue, the next session's join
     * and Stop events could be backpressured behind obsolete notifications.
     *
     * Call only while the native handle gate serializes close/rebind. The
     * previous poll job must already be cancelled, and the replacement poll
     * job must not yet be started. This drains queued items, not an event
     * already handed to a collector; that consumer still checks the handle.
     */
    fun discardRetiredEvents() = synchronized(enqueueGate) {
        while (true) {
            val retired = channel.tryReceive().getOrNull() ?: break
            if (retired.bestEffort) queuedBestEffort.decrementAndGet()
        }
    }

    /** Only use for explicitly lossy telemetry or PCM. */
    fun tryEmit(handle: Handle, event: Event) = synchronized(enqueueGate) {
        // A cancelled poll coroutine can finish an already-read native event
        // after the new handle has been installed. Do not charge its bytes or
        // diagnostic quota to the replacement session.
        if (currentHandle() !== handle) return@synchronized
        val pending = queuedBestEffort.incrementAndGet()
        if (pending > bestEffortLimit) {
            queuedBestEffort.decrementAndGet()
            dropped.incrementAndGet()
            return@synchronized
        }
        if (channel.trySend(TransportQueueEntry(handle, event, bestEffort = true)).isFailure) {
            queuedBestEffort.decrementAndGet()
            dropped.incrementAndGet()
        }
    }

    /** Required control/terminal event: wait for bounded consumer capacity. */
    suspend fun emit(handle: Handle, event: Event) {
        select<Unit> {
            channel.onSend(TransportQueueEntry(handle, event, bestEffort = false)) { }
            closed.onAwait { error("Native transport event queue closed") }
        }
    }

    override fun close() {
        // Keep queued authoritative events drainable, but never strand a
        // producer waiting for capacity after the native handle is retired.
        channel.close()
        closed.complete(Unit)
    }

    private companion object {
        const val DEFAULT_CAPACITY = 64
    }
}
