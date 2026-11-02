package com.ekkus.silentdisco.core.rust

import java.util.concurrent.atomic.AtomicLong

/**
 * Generation fence for a reusable host transport controller.
 *
 * Begin a bind before dispatching to IO; close and later binds invalidate
 * older requests immediately. The final check must run under the native
 * handle gate so a stale bind cannot reopen a socket after Stop.
 */
internal class HostTransportSessionGeneration {
    private val current = AtomicLong()

    fun beginBind(): Long = current.incrementAndGet()

    fun retire() {
        current.incrementAndGet()
    }

    fun isCurrent(generation: Long): Boolean = current.get() == generation
}
