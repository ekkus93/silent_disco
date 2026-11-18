package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.uniffi.FfiListenerBoundary
import com.ekkus.silentdisco.core.uniffi.FfiListenerBoundarySchedule
import com.ekkus.silentdisco.core.uniffi.FfiListenerBoundaryWithdrawal
import com.ekkus.silentdisco.core.uniffi.NoHandle
import java.util.TreeMap

/**
 * JVM stand-in for the Rust listener boundary schedule, which JVM tests cannot
 * load. Its rule (host time = start + sample / 48 kHz, local = host - offset; a
 * Pause takes back boundaries presented after it) is a test double of
 * `listener_boundary_schedule.rs`, which owns and tests it;
 * `ListenerBoundaryScheduleInstrumentedTest` runs the real one on a device.
 * JVM tests use this to check Kotlin's side: which events reach the schedule,
 * with which clock and offset ([takeDueCalls]), and what reaches the screen.
 */
open class FakeListenerBoundarySchedule : FfiListenerBoundarySchedule(NoHandle) {
    private var streamId: String? = null
    private var hostStartMs: Long? = null
    private var startTrack: String? = null
    private val waiting = TreeMap<Long, FfiListenerBoundary>()
    private val presented = TreeMap<Long, FfiListenerBoundary>()

    val takeDueCalls = mutableListOf<Pair<Long, Double>>()
    var forgetCalls = 0

    private fun hostTimeMs(start: Long, sample: Long) = start + sample * 1_000.0 / 48_000.0

    @Synchronized
    override fun streamStarted(streamId: String, hostStartTimeMs: ULong, startTrack: String?): Boolean {
        val same = this.streamId == streamId
        if (!same) {
            waiting.clear()
            this.streamId = streamId
        }
        hostStartMs = hostStartTimeMs.toLong()
        this.startTrack = startTrack
        presented.clear()
        return same
    }

    @Synchronized
    override fun streamStopped() {
        hostStartMs = null
        startTrack = null
        waiting.clear()
        presented.clear()
    }

    @Synchronized
    override fun forgetStream() {
        forgetCalls += 1
        streamStopped()
        streamId = null
    }

    @Synchronized
    override fun add(streamId: String, firstSampleIndex: ULong, displayName: String): Boolean {
        if (this.streamId != streamId) return false
        waiting[firstSampleIndex.toLong()] = FfiListenerBoundary(firstSampleIndex, displayName)
        return true
    }

    @Synchronized
    override fun takeDue(nowLocalMs: Long, hostOffsetMs: Double): List<FfiListenerBoundary> {
        takeDueCalls += nowLocalMs to hostOffsetMs
        val start = hostStartMs ?: return emptyList()
        val due = waiting.entries.takeWhile { hostTimeMs(start, it.key) - hostOffsetMs <= nowLocalMs }
        return due.map { (sample, boundary) ->
            waiting.remove(sample)
            presented[sample] = boundary
            boundary
        }
    }

    @Synchronized
    override fun nextDueLocalMs(hostOffsetMs: Double): Double? {
        val start = hostStartMs ?: return null
        val first = waiting.firstEntry() ?: return null
        return hostTimeMs(start, first.key) - hostOffsetMs
    }

    @Synchronized
    override fun paused(streamId: String, hostPauseTimeMs: ULong): FfiListenerBoundaryWithdrawal? {
        if (this.streamId != streamId) return null
        val start = hostStartMs ?: return null
        val withdrawn = presented.filterKeys { hostTimeMs(start, it) > hostPauseTimeMs.toDouble() }
        if (withdrawn.isEmpty()) return null
        withdrawn.forEach { (sample, boundary) ->
            presented.remove(sample)
            waiting[sample] = boundary
        }
        return FfiListenerBoundaryWithdrawal(
            count = withdrawn.size.toUInt(),
            trackAtPause = presented.lastEntry()?.value?.displayName ?: startTrack,
        )
    }

    @Synchronized
    override fun pending(): List<FfiListenerBoundary> = waiting.values.toList()

    @Synchronized
    override fun hostStartTimeMs(): ULong? = hostStartMs?.toULong()

    override fun close() = Unit
}
