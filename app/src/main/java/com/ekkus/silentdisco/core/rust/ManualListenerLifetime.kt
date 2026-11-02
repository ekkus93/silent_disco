package com.ekkus.silentdisco.core.rust

import java.util.concurrent.atomic.AtomicBoolean

/**
 * Terminal lifetime fence for the manual listener controller.
 *
 * The UI may queue a connect on Dispatchers.IO and synchronously close the
 * controller before that coroutine runs. Checking only before dispatch is
 * insufficient: the native gate must recheck [isOpen] while binding.
 * Reset is deliberately reusable; close is not.
 */
internal class ManualListenerLifetime {
    private val closed = AtomicBoolean(false)

    fun isOpen(): Boolean = !closed.get()

    fun ensureOpen() {
        check(isOpen()) { "Manual listener transport controller is closed" }
    }

    fun closeOnce(): Boolean = closed.compareAndSet(false, true)
}
