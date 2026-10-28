package com.ekkus.silentdisco.core.rust

import java.util.concurrent.atomic.AtomicReference

/**
 * Serializes native host commands against shutdown and session replacement.
 *
 * A native send can block until its TCP writer reports delivery. Merely loading
 * an [AtomicReference] before that call does not keep the handle alive: another
 * coroutine can close it, or bind a replacement session, during the send.
 * The short-lived session owner uses this gate for *all* sends and lifecycle
 * mutations. The blocking event poll deliberately stays outside the gate:
 * shutdown must be able to interrupt that poll.
 */
internal class HostNativeHandleGate<T : Any> {
    private val monitor = Any()
    private val current = AtomicReference<T?>(null)

    /** For poller identity checks only; never use this to invoke native commands. */
    fun currentHandle(): T? = current.get()

    fun <R> withCurrent(action: (T) -> R): R? = synchronized(monitor) {
        current.get()?.let(action)
    }

    /** The retire, open and poller-start sequence is one indivisible transition. */
    fun replace(
        retire: (T?) -> Unit,
        open: () -> T,
        start: (T) -> Unit,
    ) = synchronized(monitor) {
        retire(current.getAndSet(null))
        val next = open()
        current.set(next)
        try {
            start(next)
        } catch (error: Throwable) {
            current.set(null)
            retire(next)
            throw error
        }
    }

    fun close(retire: (T?) -> Unit) = synchronized(monitor) {
        retire(current.getAndSet(null))
    }
}
