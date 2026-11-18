package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiListenerBoundarySchedule
import com.ekkus.silentdisco.core.uniffi.FfiListenerBoundaryWithdrawal
import com.google.common.truth.Truth.assertThat
import org.junit.Test

/**
 * The listener's boundary schedule on the device: the real Rust object behind
 * the generated bindings, through the scenarios the Android listener relies
 * on (a reconnect replay that presents passed boundaries at once, a pause
 * that takes back boundaries presented after it, a resume that keeps them, a
 * forgotten stream). The rule is Rust's (`listener_boundary_schedule.rs`); JVM
 * tests use a test double of it.
 */
class ListenerBoundaryScheduleInstrumentedTest {
    @Test
    fun aReconnectReplayPresentsPassedBoundariesAndHoldsTheNextOne() {
        val schedule = FfiListenerBoundarySchedule()
        // Host 9 000 ms start, listener clock 10 s behind the host.
        assertThat(schedule.streamStarted("stream-1", 9_000uL, "First track")).isFalse()
        assertThat(schedule.add("stream-1", 12_000uL, "Second track")).isTrue()
        assertThat(schedule.add("stream-1", 12_000uL, "Second track")).isTrue() // replayed twice
        assertThat(schedule.add("stream-1", 240_000uL, "Third track")).isTrue()
        assertThat(schedule.add("stream-2", 1uL, "Other stream")).isFalse()

        // Local 0 is host 10 000: track two (host 9 250) has passed.
        assertThat(schedule.takeDue(0L, 10_000.0).map { it.displayName }).containsExactly("Second track")
        // Track three at host 14 000 is local 4 000.
        assertThat(schedule.nextDueLocalMs(10_000.0)).isWithin(1e-9).of(4_000.0)
        assertThat(schedule.takeDue(3_999L, 10_000.0)).isEmpty()
        assertThat(schedule.takeDue(4_000L, 10_000.0).map { it.displayName }).containsExactly("Third track")
        schedule.close()
    }

    @Test
    fun aPauseTakesBackLaterBoundariesAndAResumeKeepsThem() {
        val schedule = FfiListenerBoundarySchedule()
        schedule.streamStarted("stream-1", 0uL, "First")
        schedule.add("stream-1", 48_000uL, "Second")
        schedule.add("stream-1", 144_000uL, "Third")
        assertThat(schedule.takeDue(4_000L, 0.0).map { it.displayName }).containsExactly("Second", "Third").inOrder()

        assertThat(schedule.paused("stream-1", 2_000uL))
            .isEqualTo(FfiListenerBoundaryWithdrawal(count = 1u, trackAtPause = "Second"))
        assertThat(schedule.paused("stream-2", 0uL)).isNull()
        // The host resumes the same stream, re-anchored 60 s later.
        assertThat(schedule.streamStarted("stream-1", 60_000uL, "Second")).isTrue()
        assertThat(schedule.pending().map { it.displayName }).containsExactly("Third")
        assertThat(schedule.nextDueLocalMs(0.0)).isWithin(1e-9).of(63_000.0)

        schedule.forgetStream()
        assertThat(schedule.streamStarted("stream-1", 0uL, null)).isFalse()
        assertThat(schedule.pending()).isEmpty()
        schedule.close()
    }
}
