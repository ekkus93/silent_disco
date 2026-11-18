package com.ekkus.silentdisco.app.controller

import com.ekkus.silentdisco.app.FakeListenerBoundarySchedule
import com.ekkus.silentdisco.core.uniffi.FfiHostPlaylistException
import com.ekkus.silentdisco.core.uniffi.FfiListenerBoundary
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Test

/**
 * Kotlin's side of the listener's track boundaries: when it asks the Rust
 * schedule, with which clock and offset, and what it does with the answer.
 * When a boundary is due is Rust's rule, tested in
 * `listener_boundary_schedule_tests.rs` and on the device.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class ListenerTrackBoundariesTest {
    private val applied = mutableListOf<String>()
    private val failures = mutableListOf<Throwable>()
    private val schedule = FakeListenerBoundarySchedule()

    private inner class Context(
        override var paused: Boolean = false,
        override var hostOffsetMs: Double? = 0.0,
    ) : ListenerBoundaryContext {
        var scheduled = 0
        override fun onScheduled() {
            scheduled += 1
        }
        override fun onApplied(boundary: FfiListenerBoundary) {
            applied += boundary.displayName
        }
        override fun onFailed(error: Throwable) {
            failures += error
        }
    }

    private fun TestScope.tracker(on: FakeListenerBoundarySchedule = schedule) =
        ListenerTrackBoundaries(this, elapsedRealtimeMs = { testScheduler.currentTime }, newSchedule = { on })

    /** 48 kHz: one second of samples. */
    private val second = 48_000L

    @Test
    fun aBoundaryIsPresentedWhenRustSaysAndThePollerPassesTheCurrentOffset() = runTest {
        val boundaries = tracker()
        val context = Context(hostOffsetMs = -500.0)
        boundaries.onStreamStarted("stream-1", hostStartTimeMs = 1_000L, startTrackName = "First")
        assertThat(boundaries.add("stream-1", 2 * second, "Second")).isTrue()

        boundaries.poll(context)
        // Host time 3_000 ms is local 3_500 ms with a -500 ms offset.
        advanceTimeBy(3_499L)
        runCurrent()
        assertThat(applied).isEmpty()
        advanceTimeBy(1L)
        runCurrent()

        assertThat(applied).containsExactly("Second")
        assertThat(boundaries.pending).isEmpty()
        assertThat(context.scheduled).isEqualTo(1)
        assertThat(schedule.takeDueCalls.map { it.second }.toSet()).containsExactly(-500.0)
        assertThat(schedule.takeDueCalls.all { (now, _) -> now <= testScheduler.currentTime }).isTrue()
        advanceUntilIdle()
        assertThat(boundaries.activePoller).isNull()
    }

    @Test
    fun nothingIsAskedWhilePausedOrBeforeSync() = runTest {
        val boundaries = tracker()
        boundaries.onStreamStarted("stream-1", hostStartTimeMs = 0L, startTrackName = "First")
        boundaries.add("stream-1", second, "Second")

        boundaries.poll(Context(paused = true))
        boundaries.poll(Context(hostOffsetMs = null))
        advanceTimeBy(10_000L)
        runCurrent()

        assertThat(applied).isEmpty()
        assertThat(schedule.takeDueCalls).isEmpty()
        assertThat(boundaries.pending.map { it.firstSampleIndex.toLong() }).containsExactly(second)
    }

    /**
     * Each change restarts the poller, so a pause or a new offset is seen by
     * the next turn and never raced by a poller that has already decided.
     */
    @Test
    fun aPauseStopsThePollerAndTheNextChangeRestartsItWithTheNewOffset() = runTest {
        val boundaries = tracker()
        val context = Context()
        boundaries.onStreamStarted("stream-1", hostStartTimeMs = 0L, startTrackName = "First")
        boundaries.add("stream-1", 2 * second, "Second")
        boundaries.poll(context)
        runCurrent()

        assertThat(boundaries.onPaused("stream-1", hostPauseTimeMs = 1_000L)).isNull()
        assertThat(boundaries.activePoller).isNull()
        advanceTimeBy(5_000L)
        runCurrent()
        assertThat(applied).isEmpty()

        context.hostOffsetMs = 250.0
        boundaries.poll(context)
        runCurrent()
        // Due at host 2 000 = local 1 750, already past: presented at once.
        assertThat(applied).containsExactly("Second")
        assertThat(schedule.takeDueCalls.last().second).isEqualTo(250.0)
    }

    @Test
    fun overdueBoundariesArePresentedTogetherInOrder() = runTest {
        val boundaries = tracker()
        boundaries.onStreamStarted("stream-1", hostStartTimeMs = 0L, startTrackName = "First")
        boundaries.add("stream-1", 2 * second, "Third")
        boundaries.add("stream-1", second, "Second")
        advanceTimeBy(5_000L)
        boundaries.poll(Context())
        runCurrent()
        assertThat(applied).containsExactly("Second", "Third").inOrder()
    }

    @Test
    fun stopAndForgettingTheStreamEndThePollerAndReachTheSchedule() = runTest {
        val boundaries = tracker()
        boundaries.onStreamStarted("stream-1", hostStartTimeMs = 0L, startTrackName = "First")
        boundaries.add("stream-1", second, "Second")
        boundaries.poll(Context())
        runCurrent()

        boundaries.onStreamStopped()
        assertThat(boundaries.activePoller).isNull()
        assertThat(boundaries.streamHostStartTimeMs).isNull()
        advanceTimeBy(20_000L)
        runCurrent()
        assertThat(applied).isEmpty()

        boundaries.forgetStream()
        assertThat(schedule.forgetCalls).isEqualTo(1)
        assertThat(boundaries.add("stream-1", second, "Second")).isFalse()
    }

    @Test
    fun aRustScheduleFailureIsReportedAndEndsThePoller() = runTest {
        val failing = object : FakeListenerBoundarySchedule() {
            override fun takeDue(nowLocalMs: Long, hostOffsetMs: Double): List<FfiListenerBoundary> =
                throw FfiHostPlaylistException.Closed("listener boundary schedule is poisoned")
        }
        val boundaries = tracker(failing)
        boundaries.onStreamStarted("stream-1", hostStartTimeMs = 0L, startTrackName = "First")
        boundaries.add("stream-1", second, "Second")
        boundaries.poll(Context())
        advanceUntilIdle()

        assertThat(failures.single().message).isEqualTo("listener boundary schedule is poisoned")
        assertThat(boundaries.activePoller).isNull()
        assertThat(applied).isEmpty()
    }
}
