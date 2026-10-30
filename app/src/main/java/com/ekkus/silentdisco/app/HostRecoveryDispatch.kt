package com.ekkus.silentdisco.app

import java.util.concurrent.Semaphore
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

/**
 * Prevent clock probes from spawning unlimited pending IO continuations.
 *
 * Saturated sync probes are retriable by listeners. Resync is authoritative:
 * never discard a distinct admitted peer's recovery request; coalesce only
 * duplicate requests for the exact same peer and native host generation.
 * The native transport already bounds admitted peer count.
 */
internal class HostRecoveryDispatch(
    private val maxSyncInFlight: Int = 32,
    private val maxSyncPerListener: Int = 4,
) {
    init {
        require(maxSyncInFlight > 0)
        require(maxSyncPerListener > 0)
    }
    private val syncSlots = Semaphore(maxSyncInFlight)
    private val syncLock = Any()
    // A noisy admitted peer cannot monopolize every sync slot and starve
    // another listener's clock. Retired generations have independent quotas.
    private val syncByListener = mutableMapOf<ResyncKey, Int>()
    private val refused = AtomicLong()
    private val resyncLock = Any()
    private val activeResyncs = mutableMapOf<ResyncKey, ResyncState>()
    private class ResyncState(var replayAgain: Boolean = false)

    internal val rejectedSyncRequests: Long get() = refused.get()
    internal val activeSyncRequests: Int get() = maxSyncInFlight - syncSlots.availablePermits()
    internal val activeSyncListeners: Int get() = synchronized(syncLock) { syncByListener.size }
    internal val activeResyncRequests: Int get() = synchronized(resyncLock) { activeResyncs.size }

    fun launchSync(
        scope: CoroutineScope,
        dispatcher: CoroutineDispatcher,
        generation: Any,
        listenerId: String,
        body: suspend () -> Unit,
    ): Boolean {
        if (!syncSlots.tryAcquire()) {
            refused.incrementAndGet()
            return false
        }
        val key = ResyncKey(generation, listenerId)
        val admitted = synchronized(syncLock) {
            val current = syncByListener[key] ?: 0
            if (current >= maxSyncPerListener) {
                false
            } else {
                syncByListener[key] = current + 1
                true
            }
        }
        if (!admitted) {
            syncSlots.release()
            refused.incrementAndGet()
            return false
        }
        fun release() {
            synchronized(syncLock) {
                val remaining = (syncByListener[key] ?: 1) - 1
                if (remaining == 0) syncByListener.remove(key)
                else syncByListener[key] = remaining
            }
            syncSlots.release()
        }
        try {
            // Completion also releases both quotas when cancellation occurs
            // before the IO dispatcher starts the coroutine.
            scope.launch(dispatcher) { body() }.invokeOnCompletion { release() }
        } catch (error: Throwable) {
            release()
            throw error
        }
        return true
    }

    fun launchResync(
        scope: CoroutineScope,
        dispatcher: CoroutineDispatcher,
        generation: Any,
        listenerId: String,
        body: suspend () -> Unit,
    ): Boolean {
        val key = ResyncKey(generation, listenerId)
        val state = synchronized(resyncLock) {
            activeResyncs[key]?.let {
                // Never lose the last authoritative notice behind an in-flight
                // replay. Coalesce any number of duplicates into ONE follow-up.
                it.replayAgain = true
                return false
            }
            ResyncState().also { activeResyncs[key] = it }
        }
        fun retire() = synchronized(resyncLock) {
            if (activeResyncs[key] === state) activeResyncs.remove(key)
        }
        try {
            scope.launch(dispatcher) {
                try {
                    while (true) {
                        body()
                        val repeat = synchronized(resyncLock) {
                            if (state.replayAgain) {
                                state.replayAgain = false
                                true
                            } else {
                                // Removal and final dirty check must be atomic:
                                // a concurrent notice then starts a fresh job.
                                activeResyncs.remove(key)
                                false
                            }
                        }
                        if (!repeat) break
                    }
                } finally {
                    retire()
                }
            }.invokeOnCompletion { retire() }
        } catch (error: Throwable) {
            retire()
            throw error
        }
        return true
    }

    private class ResyncKey(val generation: Any, val listenerId: String) {
        override fun equals(other: Any?): Boolean = other is ResyncKey &&
            generation === other.generation && listenerId == other.listenerId
        override fun hashCode(): Int = 31 * System.identityHashCode(generation) + listenerId.hashCode()
    }
}
