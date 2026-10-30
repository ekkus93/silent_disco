package com.ekkus.silentdisco.core.rust

/**
 * A single owner for a lazily opened native core handle. Native callbacks and
 * commands hold the same monitor as close, so shutdown cannot release memory
 * while an already-started UniFFI invocation is still using the handle.
 *
 * Never suspend inside [withOpen] or [withExisting]. The host controller
 * releases the lease before awaiting Rust actor snapshots. Calls queued
 * after close cannot open a replacement core.
 */
internal class CoreNativeHandleGate<H : Any>(
    private val open: () -> H,
    private val retire: (H) -> Unit,
) {
    private val monitor = Any()
    private var current: H? = null
    private var closed = false

    fun <R> withOpen(action: (H) -> R): R = synchronized(monitor) {
        check(!closed) { "Rust host controller is closed" }
        val handle = current ?: open().also { current = it }
        action(handle)
    }

    fun <R> withExisting(action: (H) -> R): R? = synchronized(monitor) {
        if (closed) null else current?.let(action)
    }

    fun close() = synchronized(monitor) {
        if (closed) return@synchronized
        closed = true
        val handle = current
        current = null
        if (handle != null) retire(handle)
    }
}

/**
 * A native handle must be retired if its first snapshot/observer initialization
 * fails. An exception from initialization happens *after* UniFFI allocates the
 * native actor but before [CoreNativeHandleGate] can take ownership; without
 * this transaction the unreachable native actor would leak across retries.
 *
 * Preserve the original initialization failure even when shutdown/close also
 * fails. The gate may retry opening on a later command.
 */
internal fun <H : Any> openInitializedNativeHandle(
    open: () -> H,
    initialize: (H) -> Unit,
    retire: (H) -> Unit,
): H {
    val handle = open()
    try {
        initialize(handle)
        return handle
    } catch (failure: Throwable) {
        try {
            retire(handle)
        } catch (cleanup: Throwable) {
            if (cleanup !== failure) failure.addSuppressed(cleanup)
        }
        throw failure
    }
}
