package com.ekkus.silentdisco.app.controller

import android.os.SystemClock
import com.ekkus.silentdisco.core.uniffi.FfiListenerBoundary
import com.ekkus.silentdisco.core.uniffi.FfiListenerBoundarySchedule
import com.ekkus.silentdisco.core.uniffi.FfiListenerBoundaryWithdrawal
import kotlin.math.ceil
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/** What the boundary poller reads from the listener's current state. */
internal interface ListenerBoundaryContext {
    /** True while the host has the stream paused; no boundary is presented then. */
    val paused: Boolean

    /** The host-to-local clock offset, or null while sync is still unknown. */
    val hostOffsetMs: Double?

    /** The poller started waiting for a boundary. */
    fun onScheduled()

    /** A boundary reached its presentation time: show its track. */
    fun onApplied(boundary: FfiListenerBoundary)

    /** The Rust schedule failed; the track shown may now be wrong. */
    fun onFailed(error: Throwable)
}

/**
 * Track boundaries of the stream the listener is playing (God-file refactor
 * spec §4, ListenerPlayback). When a boundary is presented, mapped from host
 * time onto the local clock, is decided by the Rust schedule
 * ([FfiListenerBoundarySchedule], owner decision 2026-11-18; formerly Kotlin
 * timing state, spec A.4 T1). This keeps only the clock and one poller.
 *
 * Transport events arrive on an IO thread and the poller runs on [scope], so
 * every change that can make a boundary due restarts the poller through
 * [poll]: the new poller sees the change, and no poller decides to stop just
 * as a change lands.
 */
internal class ListenerTrackBoundaries(
    private val scope: CoroutineScope,
    private val elapsedRealtimeMs: () -> Long = SystemClock::elapsedRealtime,
    newSchedule: () -> FfiListenerBoundarySchedule = { FfiListenerBoundarySchedule() },
) {
    val schedule: FfiListenerBoundarySchedule = newSchedule()

    private var poller: Job? = null

    /** The running poller, if any; for tests and diagnostics. */
    val activePoller: Job? @Synchronized get() = poller?.takeIf { it.isActive }

    /** The current stream's host start time; null before a start and after Stop. */
    val streamHostStartTimeMs: Long? get() = schedule.hostStartTimeMs()?.toLong()

    /** The boundaries waiting, by first sample index. */
    val pending: List<FfiListenerBoundary> get() = schedule.pending()

    /**
     * A StreamStart. Returns whether Rust counts it as resuming the stream
     * already followed, which keeps that stream's pending boundaries.
     */
    fun onStreamStarted(streamId: String, hostStartTimeMs: Long, startTrackName: String?): Boolean =
        schedule.streamStarted(streamId, hostStartTimeMs.toULong(), startTrackName)

    fun onStreamStopped() {
        stopPolling()
        schedule.streamStopped()
    }

    /** The listener stopped following any stream (left or switched role). */
    fun forgetStream() {
        stopPolling()
        schedule.forgetStream()
    }

    /** A boundary from the host; false when it belongs to another stream. */
    fun add(streamId: String, firstSampleIndex: Long, displayName: String): Boolean =
        schedule.add(streamId, firstSampleIndex.toULong(), displayName)

    /** A host Pause: what it took back, or null when nothing was presented after it. */
    fun onPaused(streamId: String, hostPauseTimeMs: Long): FfiListenerBoundaryWithdrawal? {
        stopPolling()
        return schedule.paused(streamId, hostPauseTimeMs.toULong())
    }

    @Synchronized
    fun stopPolling() {
        poller?.cancel()
        poller = null
    }

    /**
     * Restarts the poller after a change. It ends while paused or before
     * sync, when nothing is pending, or on a Rust failure; the next change
     * starts it again.
     */
    @Synchronized
    fun poll(context: ListenerBoundaryContext) {
        poller?.cancel()
        poller = scope.launch {
            var announced = false
            while (isActive) {
                if (context.paused) return@launch
                val offsetMs = context.hostOffsetMs ?: return@launch
                val nextDueMs = try {
                    schedule.takeDue(elapsedRealtimeMs(), offsetMs).forEach { boundary ->
                        if (!isActive) return@launch
                        context.onApplied(boundary)
                    }
                    schedule.nextDueLocalMs(offsetMs)
                } catch (error: com.ekkus.silentdisco.core.uniffi.FfiHostPlaylistException) {
                    context.onFailed(error)
                    return@launch
                } ?: return@launch
                if (!announced) {
                    context.onScheduled()
                    announced = true
                }
                delay(ceil(nextDueMs - elapsedRealtimeMs()).toLong().coerceAtLeast(1L))
            }
        }
    }
}
