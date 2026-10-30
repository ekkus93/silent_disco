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
internal class HostRecoveryDispatch(private val maxSyncInFlight: Int = 32) {
    init { require(maxSyncInFlight > 0) }
    private val syncSlots = Semaphore(maxSyncInFlight)
    private val refused = AtomicLong()
    private val resyncLock = Any()
    private val activeResyncs = mutableMapOf<ResyncKey, ResyncState>()
    private class ResyncState(var replayAgain: Boolean = false)

    internal val rejectedSyncRequests: Long get() = refused.get()
    internal val activeSyncRequests: Int get() = maxSyncInFlight - syncSlots.availablePermits()
    internal val activeResyncRequests: Int get() = synchronized(resyncLock) { activeResyncs.size }

    fun launchSync(
        scope: CoroutineScope,
        dispatcher: CoroutineDispatcher,
        body: suspend () -> Unit,
    ): Boolean {
        if (!syncSlots.tryAcquire()) {
            refused.incrementAndGet()
            return false
        }
        try {
            // invokeOnCompletion also releases a slot when cancellation occurs
            // before the dispatcher starts the coroutine at all.
            scope.launch(dispatcher) { body() }.invokeOnCompletion { syncSlots.release() }
        } catch (error: Throwable) {
            syncSlots.release()
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
