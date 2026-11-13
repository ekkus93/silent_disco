package com.ekkus.silentdisco.app.controller

import com.ekkus.silentdisco.app.PendingListenerTrackBoundary
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class ListenerTrackBoundariesTest {
    private val applied = mutableListOf<String>()

    private inner class Context(
        override var paused: Boolean = false,
        override var hostOffsetMs: Double? = 0.0,
        override var currentStreamId: String? = "stream-1",
    ) : ListenerBoundaryContext {
        var scheduled = 0
        override fun onScheduled() {
            scheduled += 1
        }
        override fun onApplied(boundary: PendingListenerTrackBoundary) {
            applied += boundary.displayName
        }
    }

    /** 48 kHz: sample index 48_000 is one second after the stream's host start. */
    private fun boundary(seconds: Long, name: String, stream: String = "stream-1") =
        PendingListenerTrackBoundary(stream, firstSampleIndex = seconds * 48_000L, displayName = name)

    private fun TestScope.tracker() = ListenerTrackBoundaries(this, elapsedRealtimeMs = { testScheduler.currentTime })

    @Test
    fun aBoundaryIsAppliedAtItsLocalPresentationTime() = runTest {
        val boundaries = tracker()
        val context = Context(hostOffsetMs = -500.0)
        boundaries.onStreamStarted(hostStartTimeMs = 1_000L, startTrackName = "First", sameStream = false)
        boundaries.add(boundary(seconds = 2, name = "Second"))

        boundaries.schedule(96_000L, context)
        // Host time 3_000 ms is local 3_500 ms with a -500 ms offset.
        advanceTimeBy(3_499L)
        runCurrent()
        assertThat(applied).isEmpty()
        advanceTimeBy(2L)
        runCurrent()

        assertThat(applied).containsExactly("Second")
        assertThat(boundaries.pending).isEmpty()
        assertThat(boundaries.applied.keys).containsExactly(96_000L)
        assertThat(context.scheduled).isEqualTo(1)
    }

    @Test
    fun nothingIsScheduledWhilePausedOrBeforeSync() = runTest {
        val boundaries = tracker()
        boundaries.onStreamStarted(hostStartTimeMs = 0L, startTrackName = "First", sameStream = false)
        boundaries.add(boundary(seconds = 1, name = "Second"))

        boundaries.schedule(48_000L, Context(paused = true))
        boundaries.schedule(48_000L, Context(hostOffsetMs = null))
        advanceTimeBy(10_000L)
        runCurrent()

        assertThat(applied).isEmpty()
        assertThat(boundaries.pending.keys).containsExactly(48_000L)
    }

    @Test
    fun aBoundaryOfAnotherStreamOrOnePausedBeforeItsTimeIsNotApplied() = runTest {
        val boundaries = tracker()
        val context = Context()
        boundaries.onStreamStarted(hostStartTimeMs = 0L, startTrackName = "First", sameStream = false)
        boundaries.add(boundary(seconds = 1, name = "Second"))
        boundaries.schedule(48_000L, context)

        context.paused = true
        advanceTimeBy(2_000L)
        runCurrent()

        assertThat(applied).isEmpty()
        assertThat(boundaries.pending.keys).containsExactly(48_000L)

        context.paused = false
        boundaries.reschedule(context)
        context.currentStreamId = "stream-2"
        advanceTimeBy(2_000L)
        runCurrent()
        assertThat(applied).isEmpty()
    }

    @Test
    fun aPauseWithdrawsBoundariesPresentedAfterItAndRestoresTheTrackAtThePause() = runTest {
        val boundaries = tracker()
        val context = Context()
        boundaries.onStreamStarted(hostStartTimeMs = 0L, startTrackName = "First", sameStream = false)
        boundaries.add(boundary(seconds = 1, name = "Second"))
        boundaries.add(boundary(seconds = 3, name = "Third"))
        boundaries.reschedule(context)
        advanceTimeBy(4_000L)
        runCurrent()
        assertThat(applied).containsExactly("Second", "Third").inOrder()

        assertThat(boundaries.withdrawAfter(hostPauseTimeMs = 2_000L)).isEqualTo(1 to "Second")
        assertThat(boundaries.pending.keys).containsExactly(144_000L)

        assertThat(boundaries.withdrawAfter(hostPauseTimeMs = 500L)).isEqualTo(1 to "First")
        assertThat(boundaries.withdrawAfter(hostPauseTimeMs = 500L)).isNull()
    }

    @Test
    fun aResumedStreamKeepsItsPendingBoundariesAndANewOneDropsThem() = runTest {
        val boundaries = tracker()
        boundaries.onStreamStarted(hostStartTimeMs = 0L, startTrackName = "First", sameStream = false)
        boundaries.add(boundary(seconds = 1, name = "Second"))

        boundaries.onStreamStarted(hostStartTimeMs = 5_000L, startTrackName = "First", sameStream = true)
        assertThat(boundaries.pending.keys).containsExactly(48_000L)

        boundaries.onStreamStarted(hostStartTimeMs = 9_000L, startTrackName = "Other", sameStream = false)
        assertThat(boundaries.pending).isEmpty()

        boundaries.add(boundary(seconds = 1, name = "Next"))
        boundaries.reschedule(Context())
        boundaries.onStreamStopped()
        advanceTimeBy(20_000L)
        runCurrent()
        assertThat(applied).isEmpty()
        assertThat(boundaries.streamHostStartTimeMs).isNull()
        assertThat(boundaries.timers).isEmpty()
    }
}
