package com.ekkus.silentdisco.app

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class StorageMappingTest {
    @Test
    fun tuningSettingsRoundTripThroughTheRustDtoWithoutDroppingScanWindow() {
        val original = TuningSettings(
            syncSampleWindow = 18,
            syncCadenceMs = 2_750L,
            startupBufferMs = 550L,
            latePacketThresholdMs = 55L,
            hardResyncThresholdMs = 180L,
            syncDriftThresholdMs = 22.5,
            scanWindowMs = 6_500L,
        )

        val stored = original.toRustStoredSettings(updatedAtMs = 99_000L)

        assertThat(stored.updatedAtMs).isEqualTo(99_000L)
        assertThat(stored.toTuningSettings()).isEqualTo(original)
    }
}
