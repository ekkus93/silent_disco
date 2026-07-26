package com.ekkus.silentdisco.app

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class TuningResetTest {
    @Test
    fun resetDefaultsReplacesEveryTuningValueInOneCommand() {
        val modified = TuningSettings(
            syncSampleWindow = 30,
            syncCadenceMs = 4_750L,
            startupBufferMs = 1_450L,
            latePacketThresholdMs = 180L,
            hardResyncThresholdMs = 480L,
            syncDriftThresholdMs = 96.0,
            scanWindowMs = 9_500L,
        )

        val reset = modified.adjust(TuningField.ResetDefaults, direction = 0)

        assertThat(reset).isEqualTo(TuningSettings())
    }

    @Test
    fun resetDefaultsIsIdempotent() {
        val defaults = TuningSettings()

        assertThat(defaults.adjust(TuningField.ResetDefaults, direction = 1)).isEqualTo(defaults)
        assertThat(defaults.adjust(TuningField.ResetDefaults, direction = -1)).isEqualTo(defaults)
    }
}
