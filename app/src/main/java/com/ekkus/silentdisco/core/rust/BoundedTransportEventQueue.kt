package com.ekkus.silentdisco.core.rust

import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.receiveAsFlow

/**
 * Bounded native-poller-to-Android bridge. Critical events suspend only the
 * IO poll coroutine until the collector accepts them; they are never evicted
 * by a buffer overflow. High-rate telemetry/PCM uses explicit best-effort
 * admission and increments a diagnostic counter when full.
 *
 * Each item retains its native handle identity. A rebind can therefore never
 * deliver queued events from a retired generation to the next session.
 * The producer must not be a real-time audio callback.
 */
internal class BoundedTransportEventQueue<Handle : Any, Event : Any>(
    private val currentHandle: () -> Handle?,
    capacity: Int = DEFAULT_CAPACITY,
) : AutoCloseable {
    private val channel = Channel<Pair<Handle, Event>>(capacity)
    private val dropped = AtomicLong()

    val events: Flow<Event> = channel.receiveAsFlow()
        .filter { (handle, _) -> currentHandle() === handle }
        .map { (_, event) -> event }

    val droppedBestEffort: Long get() = dropped.get()

    /** Only use for explicitly lossy telemetry or PCM. */
    fun tryEmit(handle: Handle, event: Event) {
        if (channel.trySend(handle to event).isFailure) {
            dropped.incrementAndGet()
        }
    }

    /** Required control/terminal event: wait for bounded consumer capacity. */
    suspend fun emit(handle: Handle, event: Event) {
        channel.send(handle to event)
    }

    override fun close() {
        channel.close()
    }

    private companion object {
        const val DEFAULT_CAPACITY = 64
    }
}
