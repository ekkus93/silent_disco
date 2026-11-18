package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.uniffi.FfiHostBoundary
import com.ekkus.silentdisco.core.uniffi.FfiHostBoundarySchedule
import com.ekkus.silentdisco.core.uniffi.FfiHostPlaylistException
import com.ekkus.silentdisco.core.uniffi.NoHandle

/**
 * JVM stand-in for the Rust host boundary schedule, which JVM tests cannot
 * load. The deadline rule it applies (start + sample time rounded down + pause
 * offset) is a test double: the rule is owned and tested in Rust
 * (`host_boundary_schedule_tests.rs`, the FFI combined Android/desktop
 * scenario) and run for real by `HostBoundaryScheduleInstrumentedTest`. What
 * JVM tests check through this fake is Kotlin's side: the clock and pause
 * offset it passes ([takeDueCalls]), that it does not ask while paused, and
 * what it does with what comes back.
 */
open class FakeHostBoundarySchedule(
    val hostStartTimeMs: ULong,
    val sampleRate: UInt,
) : FfiHostBoundarySchedule(NoHandle) {
    private val waiting = ArrayDeque<FfiHostBoundary>()

    /** Every `takeDue(nowMs, pauseOffsetMs)` Kotlin made, in order. */
    val takeDueCalls = mutableListOf<Pair<Long, Long>>()
    val withdrawCalls = mutableListOf<Long>()

    private fun dueAt(boundary: FfiHostBoundary, pauseOffsetMs: ULong): ULong =
        hostStartTimeMs + boundary.firstSampleIndex * 1_000uL / sampleRate.toULong() + pauseOffsetMs

    @Synchronized
    override fun schedule(entryId: String, firstSampleIndex: ULong, displayName: String) {
        val last = waiting.lastOrNull()
        if (last != null && firstSampleIndex <= last.firstSampleIndex) {
            throw FfiHostPlaylistException.InvalidConfiguration("out of order")
        }
        waiting.addLast(FfiHostBoundary(entryId, firstSampleIndex, displayName))
    }

    @Synchronized
    override fun takeDue(nowMs: ULong, pauseOffsetMs: ULong): List<FfiHostBoundary> {
        takeDueCalls += nowMs.toLong() to pauseOffsetMs.toLong()
        val due = mutableListOf<FfiHostBoundary>()
        while (waiting.firstOrNull()?.let { dueAt(it, pauseOffsetMs) <= nowMs } == true) {
            due += waiting.removeFirst()
        }
        return due
    }

    @Synchronized
    override fun nextDueAtMs(pauseOffsetMs: ULong): ULong? = waiting.firstOrNull()?.let { dueAt(it, pauseOffsetMs) }

    @Synchronized
    override fun withdrawFrom(firstUnsentSampleIndex: ULong): List<FfiHostBoundary> {
        withdrawCalls += firstUnsentSampleIndex.toLong()
        val withdrawn = waiting.filter { it.firstSampleIndex >= firstUnsentSampleIndex }
        waiting.removeAll(withdrawn)
        return withdrawn
    }

    @Synchronized
    override fun pending(): List<FfiHostBoundary> = waiting.toList()

    @Synchronized
    override fun clear() = waiting.clear()

    override fun close() = Unit
}

/** Records every schedule a [MainViewModel] opens, newest last. */
class FakeHostBoundaryScheduleFactory : (ULong, UInt) -> FfiHostBoundarySchedule {
    val opened = mutableListOf<FakeHostBoundarySchedule>()
    val latest: FakeHostBoundarySchedule get() = opened.last()

    /** Handed out by the next open instead of a fresh fake. */
    var next: FakeHostBoundarySchedule? = null

    override fun invoke(hostStartTimeMs: ULong, sampleRate: UInt): FfiHostBoundarySchedule {
        val schedule = next?.also { next = null } ?: FakeHostBoundarySchedule(hostStartTimeMs, sampleRate)
        opened += schedule
        return schedule
    }
}
