package com.ekkus.silentdisco.core.rust

import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.selects.select
import kotlinx.coroutines.withTimeoutOrNull

/**
 * Finite Rust-actor-to-Android bridge. Snapshots are separately conflated in
 * StateFlow; only disposable diagnostics may be dropped under pressure.
 * Critical effects use bounded backpressure on the Rust observer thread,
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

    private val channel = Channel<Event>(capacity)
    private val closed = CompletableDeferred<Unit>()
    private val dropped = AtomicLong()
    private val timedOut = AtomicLong()

    val events: Flow<Event> = channel.receiveAsFlow()
    val droppedDiagnostics: Long get() = dropped.get()
    val criticalTimeouts: Long get() = timedOut.get()

    fun publishBestEffort(event: Event) {
        if (channel.trySend(event).isFailure) dropped.incrementAndGet()
    }

    fun publishCritical(event: Event) {
        if (channel.trySend(event).isSuccess) return
        val delivered = runBlocking {
            withTimeoutOrNull(criticalTimeoutMs) {
                select<Boolean> {
                    channel.onSend(event) { true }
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
