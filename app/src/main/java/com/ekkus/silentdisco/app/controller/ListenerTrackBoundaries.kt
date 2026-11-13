package com.ekkus.silentdisco.app.controller

import android.os.SystemClock
import com.ekkus.silentdisco.app.PendingListenerTrackBoundary
import com.ekkus.silentdisco.app.listenerBoundaryHostTimeMs
import com.ekkus.silentdisco.app.listenerBoundaryLocalTimeMs
import java.util.SortedMap
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

private const val MAX_APPLIED_LISTENER_TRACK_BOUNDARIES = 100

/** What the boundary timers read from the listener's current state. */
internal interface ListenerBoundaryContext {
    /** True while the host has the stream paused; no boundary is presented then. */
    val paused: Boolean

    /** The host-to-local clock offset, or null while sync is still unknown. */
    val hostOffsetMs: Double?

    /** The stream the listener is following now. */
    val currentStreamId: String?

    /** A timer was set for a boundary. */
    fun onScheduled()

    /** A boundary reached its presentation time: show its track. */
    fun onApplied(boundary: PendingListenerTrackBoundary)
}

/**
 * Track boundaries of the stream the listener is playing (God-file refactor
 * spec §4, ListenerPlayback). Each boundary waits as pending until its
 * presentation time, mapped onto the local clock, then is applied so the
 * screen shows its track. This is Kotlin timing state (spec A.4 T1), logged
 * for a later move into Rust.
 */
internal class ListenerTrackBoundaries(
    private val scope: CoroutineScope,
    private val elapsedRealtimeMs: () -> Long = SystemClock::elapsedRealtime,
) {
    /** Host start time of the current stream, which boundary sample indexes count from. */
    var streamHostStartTimeMs: Long? = null

    val pending = linkedMapOf<Long, PendingListenerTrackBoundary>()

    /**
     * Boundaries presented since the last StreamStart, and the track that
     * StreamStart announced. A host Pause undoes any presented past its pause
     * instant: a replayed paused stream carries its pre-pause anchor, so its
     * upcoming boundary otherwise looks overdue before the Pause arrives.
     */
    val applied: SortedMap<Long, PendingListenerTrackBoundary> = sortedMapOf()
    private var streamStartTrackName: String? = null

    val timers = mutableMapOf<Long, Job>()

    /** A StreamStart: a new stream drops the old stream's pending boundaries; a resumed one keeps them. */
    fun onStreamStarted(hostStartTimeMs: Long, startTrackName: String?, sameStream: Boolean) {
        streamHostStartTimeMs = hostStartTimeMs
        streamStartTrackName = startTrackName
        applied.clear()
        if (!sameStream) pending.clear()
    }

    fun onStreamStopped() {
        streamHostStartTimeMs = null
        pending.clear()
        applied.clear()
        streamStartTrackName = null
        cancelTimers()
    }

    fun add(boundary: PendingListenerTrackBoundary) {
        pending[boundary.firstSampleIndex] = boundary
    }

    /**
     * Returns boundaries presented after the host's pause instant to pending.
     * Returns how many moved and the track playing at the pause, or null when
     * none did. The host holds such boundaries across the pause and re-sends
     * them, re-anchored, on resume.
     */
    fun withdrawAfter(hostPauseTimeMs: Long): Pair<Int, String?>? {
        val hostStartMs = streamHostStartTimeMs ?: return null
        val withdrawn = applied.filterKeys { sampleIndex ->
            listenerBoundaryHostTimeMs(hostStartMs, sampleIndex) > hostPauseTimeMs
        }
        if (withdrawn.isEmpty()) return null
        withdrawn.forEach { (sampleIndex, boundary) ->
            applied.remove(sampleIndex)
            pending[sampleIndex] = boundary
        }
        return withdrawn.size to (applied.values.lastOrNull()?.displayName ?: streamStartTrackName)
    }

    fun cancelTimers() {
        timers.values.forEach { it.cancel() }
        timers.clear()
    }

    fun reschedule(context: ListenerBoundaryContext) {
        cancelTimers()
        pending.keys.toList().forEach { schedule(it, context) }
    }

    fun schedule(sampleIndex: Long, context: ListenerBoundaryContext) {
        if (context.paused) return
        val offsetMs = context.hostOffsetMs ?: return
        val boundary = pending[sampleIndex] ?: return
        val hostStartMs = streamHostStartTimeMs ?: return
        val boundaryLocalTimeMs = listenerBoundaryLocalTimeMs(hostStartMs, sampleIndex, offsetMs)
        val waitMs = (boundaryLocalTimeMs - elapsedRealtimeMs()).coerceAtLeast(0.0).toLong()
        timers.remove(sampleIndex)?.cancel()
        context.onScheduled()
        timers[sampleIndex] = scope.launch {
            delay(waitMs)
            if (context.currentStreamId != boundary.streamId) return@launch
            if (context.paused) return@launch
            pending.remove(sampleIndex)
            timers.remove(sampleIndex)
            applied[sampleIndex] = boundary
            while (applied.size > MAX_APPLIED_LISTENER_TRACK_BOUNDARIES) {
                applied.remove(applied.firstKey())
            }
            context.onApplied(boundary)
        }
    }
}
