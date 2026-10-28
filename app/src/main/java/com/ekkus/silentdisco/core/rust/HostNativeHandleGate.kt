package com.ekkus.silentdisco.core.rust

import java.util.concurrent.atomic.AtomicReference

/**
 * Serializes native transport commands against shutdown and session replacement.
 *
 * A native send can block until its TCP writer reports delivery. Merely loading
 * an [AtomicReference] before that call does not keep the handle alive: another
 * coroutine can close it, or bind a replacement session, during the send.
 * Both Android host and listener use this gate for *all* sends and lifecycle
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

    /** A queued command cannot silently migrate to a newly bound session. */
    fun <R> withExpected(expected: T, action: (T) -> R): R? = synchronized(monitor) {
        current.get()?.takeIf { it === expected }?.let(action)
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
            try {
                retire(next)
            } catch (cleanup: Throwable) {
                if (cleanup !== error) error.addSuppressed(cleanup)
            }
            throw error
        }
    }

    fun close(retire: (T?) -> Unit) = synchronized(monitor) {
        retire(current.getAndSet(null))
    }
}

/**
 * Approval delivery and UDP-route authorization are one operation on the SAME
 * native generation. A suspend between these calls used to allow an old
 * approval to authorize a newly rebound host handle.
 *
 * A failed authorization is never reported as admission success. Attempt to
 * retire the peer on the same handle before releasing the generation gate.
 */
internal data class HostJoinAuthorizationResult<D>(
    val delivery: D,
    val authorized: Boolean,
    val failure: String? = null,
)

internal fun <H : Any, D : Any> HostNativeHandleGate<H>.deliverAndAuthorize(
    expected: H,
    deliver: (H) -> D,
    wasDelivered: (D) -> Boolean,
    authorize: (H) -> Unit,
    disconnect: (H) -> Unit,
): HostJoinAuthorizationResult<D>? = withExpected(expected) { handle ->
    val delivery = deliver(handle)
    if (!wasDelivered(delivery)) {
        HostJoinAuthorizationResult(delivery, authorized = false)
    } else {
        try {
            authorize(handle)
            HostJoinAuthorizationResult(delivery, authorized = true)
        } catch (error: Exception) {
            runCatching { disconnect(handle) }
            HostJoinAuthorizationResult(
                delivery, authorized = false,
                failure = error.message ?: "Listener route authorization failed",
            )
        }
    }
}


/**
 * A failed shutdown must never skip the native close, and a close failure
 * must not hide the original shutdown failure. This is the host-side mirror
 * of the listener's explicit all-steps teardown contract.
 */
internal fun <H : Any> retireNativeHandle(
    handle: H?,
    shutdown: (H) -> Unit,
    close: (H) -> Unit,
) {
    if (handle == null) return
    var failure: Throwable? = null
    try {
        shutdown(handle)
    } catch (error: Throwable) {
        failure = error
    }
    try {
        close(handle)
    } catch (error: Throwable) {
        val first = failure
        if (first == null) {
            failure = error
        } else if (first !== error) {
            first.addSuppressed(error)
        }
    }
    failure?.let { throw it }
}

/** Reuse the same lifetime and generation guarantee for listener FFI handles. */
internal typealias ListenerNativeHandleGate<T> = HostNativeHandleGate<T>
