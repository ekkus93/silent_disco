package com.ekkus.silentdisco.core.rust

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlin.coroutines.CoroutineContext
import kotlin.coroutines.EmptyCoroutineContext

/**
 * Owns the one busy-join retry a listener may have waiting (remediation F3,
 * race-tested under §7). Rust decides whether and when to retry; this only
 * makes sure a retry that is no longer wanted never connects.
 *
 * Each [schedule] starts a new generation and invalidates the previous one;
 * [cancel] invalidates the current one. After its delay a retry runs only if
 * its generation is still current, and claiming that is atomic with [cancel],
 * so a cancel from another thread at the moment the delay ends cannot let a
 * stale retry through.
 */
class PendingBusyRetry {
    private val lock = Any()
    private var generation = 0L
    private var pendingGeneration: Long? = null
    private var job: Job? = null

    /**
     * True from [schedule] until the retry starts connecting or is cancelled.
     * While true, the close of the refused connection is expected.
     */
    val isPending: Boolean
        get() = synchronized(lock) { pendingGeneration != null }

    /**
     * Replaces any waiting retry: runs [beforeWait] (retire the refused
     * connection), waits [delayMs], then runs [retry] unless cancelled or
     * replaced meanwhile.
     */
    fun schedule(
        scope: CoroutineScope,
        delayMs: Long,
        context: CoroutineContext = EmptyCoroutineContext,
        beforeWait: suspend () -> Unit = {},
        retry: suspend () -> Unit,
    ) {
        val previous: Job?
        val mine: Long
        synchronized(lock) {
            previous = job
            mine = ++generation
            pendingGeneration = mine
        }
        previous?.cancel()
        val launched = scope.launch(context) {
            beforeWait()
            delay(delayMs)
            if (claim(mine)) retry()
        }
        val superseded = synchronized(lock) {
            when {
                // Still waiting: it is the job a cancel or a newer retry stops.
                pendingGeneration == mine -> {
                    job = launched
                    false
                }
                // Already claimed and connecting: nothing may cancel it now.
                generation == mine -> false
                // Cancelled or replaced before it was recorded.
                else -> true
            }
        }
        if (superseded) launched.cancel()
    }

    /**
     * Drops the waiting retry, if any; it will never connect. Returns true when
     * a retry was waiting, false when there was none or it had already started.
     */
    fun cancel(): Boolean {
        val previous: Job?
        val dropped: Boolean
        synchronized(lock) {
            generation++
            dropped = pendingGeneration != null
            pendingGeneration = null
            previous = job
            job = null
        }
        previous?.cancel()
        return dropped
    }

    private fun claim(mine: Long): Boolean = synchronized(lock) {
        if (pendingGeneration != mine) return false
        pendingGeneration = null
        job = null
        true
    }
}
