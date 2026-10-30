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
