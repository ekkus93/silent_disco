package com.ekkus.silentdisco.core.rust

import java.util.IdentityHashMap
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import java.util.concurrent.locks.ReentrantLock
import kotlin.concurrent.withLock

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
    // Polls must run without the command lock so shutdown can interrupt them.
    // Their native handles nevertheless remain live until every poll returns.
    private val pollLock = ReentrantLock()
    private val pollsDrained = pollLock.newCondition()
    private val activePolls = IdentityHashMap<T, Int>()
    // A poll may ignore shutdown beyond the bounded drain deadline. Keep the
    // retired native handle strongly reachable and release it only when its
    // final poll lease exits. Never free a handle still used by native code.
    private val deferredCloses = IdentityHashMap<T, () -> Unit>()

    /** For poller identity checks only; never use this to invoke native commands. */
    fun currentHandle(): T? = current.get()

    fun <R> withCurrent(action: (T) -> R): R? = synchronized(monitor) {
        current.get()?.let(action)
    }

    /** A queued command cannot silently migrate to a newly bound session. */
    fun <R> withExpected(expected: T, action: (T) -> R): R? = synchronized(monitor) {
        current.get()?.takeIf { it === expected }?.let(action)
    }

    /**
     * Acquire a native poll lease only for the exact current generation.
     * Unlike a command, the blocking poll runs OUTSIDE the command monitor:
     * shutdown must remain able to interrupt it. Retirement first clears
     * [current], then shuts down the socket and awaits these leases before
     * releasing the UniFFI handle. Identity comparison prevents an old poll
     * from being counted against a newly rebound session.
     */
    fun <R> withExpectedPoll(expected: T, action: (T) -> R): R? {
        pollLock.withLock {
            if (current.get() !== expected) return null
            activePolls[expected] = (activePolls[expected] ?: 0) + 1
        }
        try {
            return action(expected)
        } finally {
            val deferredClose = pollLock.withLock {
                val remaining = (activePolls[expected] ?: error("Native poll lease missing")) - 1
                if (remaining == 0) {
                    activePolls.remove(expected)
                    pollsDrained.signalAll()
                    deferredCloses.remove(expected)
                } else {
                    activePolls[expected] = remaining
                    null
                }
            }
            // Closing under pollLock would deadlock if native cleanup reenters
            // the gate. The registered callback handles and reports failures.
            deferredClose?.invoke()
        }
    }

    /**
     * Quarantine a retired handle after bounded poll-drain failure. The final
     * poll lease owns its close; if the poll finished just before registration,
     * close immediately. The failure callback must not throw.
     */
    fun deferCloseUntilPollDrained(
        retired: T,
        close: () -> Unit,
        onFailure: (Throwable) -> Unit,
    ) {
        val finish = {
            try {
                close()
            } catch (error: Throwable) {
                onFailure(error)
            }
        }
        val closeNow = pollLock.withLock {
            if (activePolls.containsKey(retired)) {
                check(!deferredCloses.containsKey(retired)) { "Retired handle already quarantined" }
                deferredCloses[retired] = finish
                false
            } else {
                true
            }
        }
        if (closeNow) finish()
    }

    /**
     * Call after native shutdown but before native close, never on an audio
     * callback. Native poll normally has a 500-ms timeout; if it hangs despite
     * shutdown, fail in bounded time WITHOUT freeing a still-in-use handle.
     * The caller must surface the error rather than treating teardown as clean.
     */
    fun awaitPollQuiescence(retired: T, timeoutMillis: Long = 5_000) {
        require(timeoutMillis > 0) { "poll drain deadline must be positive" }
        pollLock.withLock {
            var remaining = TimeUnit.MILLISECONDS.toNanos(timeoutMillis)
            while (activePolls.containsKey(retired)) {
                if (remaining <= 0) {
                    throw IllegalStateException("Native transport poll did not exit after shutdown")
                }
                remaining = pollsDrained.awaitNanos(remaining)
            }
        }
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
    awaitPoll: (H) -> Unit = {},
    deferClose: ((H, () -> Unit) -> Unit)? = null,
) {
    if (handle == null) return
    var failure: Throwable? = null
    try {
        shutdown(handle)
    } catch (error: Throwable) {
        failure = error
    }
    // Shutdown is allowed to race a blocking poll; close is not. If a poll
    // cannot be joined, fail visibly and retain the native object rather
    // than releasing memory still accessed by a native worker.
    try {
        awaitPoll(handle)
    } catch (error: Throwable) {
        // The native poll may still be accessing this handle. Do not close
        // synchronously, but arrange eventual close when its last lease exits.
        try {
            deferClose?.invoke(handle) { close(handle) }
        } catch (cleanup: Throwable) {
            if (cleanup !== error) error.addSuppressed(cleanup)
        }
        val first = failure
        if (first != null && first !== error) first.addSuppressed(error)
        throw first ?: error
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
