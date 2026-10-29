package com.ekkus.silentdisco.core.rust

import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.flow.Flow

/**
 * Bounded listener Rust-actor notification bridge.
 *
 * Snapshots are conflated by the owner into StateFlow; diagnostics are lossy.
 * Join/approval/Stop, platform effects and terminal failures are critical:
 * they receive finite backpressure on the Rust observer thread, not on a
 * native audio callback. Saturation fails explicitly rather than reporting
 * a successful operation whose notification the UI never receives.
 */
internal class ListenerCoreNotificationMailbox<Event : Any>(
    private val onSnapshot: (Event) -> Boolean,
    private val isDiagnostic: (Event) -> Boolean,
    capacity: Int = 64,
    criticalTimeoutMs: Long = 2_000L,
) : AutoCloseable {
    private val closed = AtomicBoolean(false)
    private val queue = BoundedCoreNotificationQueue<Event>(capacity, criticalTimeoutMs)

    val events: Flow<Event> = queue.events
    val droppedDiagnostics: Long get() = queue.droppedDiagnostics
    val criticalTimeouts: Long get() = queue.criticalTimeouts

    fun publish(event: Event) {
        if (closed.get()) return
        if (onSnapshot(event)) return
        if (isDiagnostic(event)) {
            queue.publishBestEffort(event)
        } else {
            queue.publishCritical(event)
        }
    }

    override fun close() {
        if (closed.compareAndSet(false, true)) queue.close()
    }
}
