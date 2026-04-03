package com.ekkus.silentdisco.core.sync

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class ClockSyncEstimatorTest {
    @Test
    fun `estimator favors low rtt samples`() {
        val estimator = ClockSyncEstimator()
        estimator.observe(ClockSyncSample(0, 10, 14, 20))
        estimator.observe(ClockSyncSample(100, 112, 116, 124))
        estimator.observe(ClockSyncSample(200, 225, 230, 260))

        val state = estimator.snapshot()

        assertThat(state.rttMs).isAtMost(20.0)
        assertThat(state.offsetMs).isWithin(0.1).of(2.0)
    }
}
