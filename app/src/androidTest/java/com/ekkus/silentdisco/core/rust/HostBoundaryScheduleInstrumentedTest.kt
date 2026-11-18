package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiHostBoundarySchedule
import com.ekkus.silentdisco.core.uniffi.FfiHostPlaylistException
import com.google.common.truth.Truth.assertThat
import org.junit.Assert.assertThrows
import org.junit.Test

/**
 * The Android host's boundary schedule on the device: the real Rust object
 * behind the generated Kotlin bindings, driven with the same script as the
 * FFI combined Android/desktop scenario
 * (`host_boundary_schedule_tests.rs::the_android_and_desktop_hosts_apply_the_same_boundaries_at_the_same_times`).
 * Same inputs, same boundaries back: the deadline rule exists once, in Rust
 * (gapless TODO §5).
 */
class HostBoundaryScheduleInstrumentedTest {
    private val start = 1_000_000uL

    @Test
    fun theDeviceScheduleGivesTheSameBoundariesAsTheSharedScenario() {
        val schedule = FfiHostBoundarySchedule(start, 48_000u)
        fun poll(nowMs: ULong, offsetMs: ULong) = schedule.takeDue(start + nowMs, offsetMs).map { it.entryId }
        fun add(entryId: String, sample: ULong) = schedule.schedule(entryId, sample, "$entryId title")

        add("track-2", 240_241uL) // 5 005.020 ms: not on a packet edge
        assertThat(poll(5_004uL, 0uL)).isEmpty()
        assertThat(poll(5_005uL, 1_500uL)).isEmpty() // a 1 500 ms pause moved it
        assertThat(poll(6_504uL, 1_500uL)).isEmpty()
        assertThat(schedule.nextDueAtMs(1_500uL)).isEqualTo(start + 6_505uL)
        assertThat(poll(6_505uL, 1_500uL)).containsExactly("track-2")

        add("track-3", 480_000uL)
        add("track-4", 528_000uL)
        assertThat(poll(11_499uL, 1_750uL)).isEmpty() // a second pause of 250 ms
        assertThat(poll(13_000uL, 1_750uL)).containsExactly("track-3", "track-4").inOrder()

        add("track-5", 576_000uL)
        add("track-6", 624_000uL)
        assertThat(schedule.withdrawFrom(576_000uL).map { it.entryId })
            .containsExactly("track-5", "track-6").inOrder()
        assertThat(schedule.takeDue(ULong.MAX_VALUE, 1_750uL)).isEmpty()
        schedule.close()
    }

    @Test
    fun theDeviceScheduleRefusesAnOutOfOrderBoundaryAndAZeroRate() {
        assertThrows(FfiHostPlaylistException.InvalidConfiguration::class.java) {
            FfiHostBoundarySchedule(start, 0u)
        }
        val schedule = FfiHostBoundarySchedule(start, 48_000u)
        schedule.schedule("a", 960uL, "A")
        assertThrows(FfiHostPlaylistException.InvalidConfiguration::class.java) {
            schedule.schedule("b", 960uL, "B")
        }
        assertThat(schedule.pending().map { it.entryId }).containsExactly("a")
        schedule.clear()
        assertThat(schedule.pending()).isEmpty()
        assertThat(schedule.nextDueAtMs(0uL)).isNull()
        schedule.close()
    }
}
