package com.ekkus.silentdisco.app

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class HostQueueBoundarySchedulerTest {
    @Test
    fun pauseShiftsScheduledBoundaryByThePausedDuration() = runTest {
        val scheduler = testScheduler
        var appliedAtMs: Long? = null
        launch {
            appliedAtMs = awaitHostQueueBoundary(
                dueAtElapsedRealtimeMs = 100L,
                isPaused = { scheduler.currentTime in 40L until 100L },
                elapsedRealtimeMs = { scheduler.currentTime },
            )
        }

        runCurrent()
        advanceTimeBy(159L)
        runCurrent()
        assertThat(appliedAtMs).isNull()

        advanceTimeBy(1L)
        runCurrent()
        assertThat(appliedAtMs).isEqualTo(160L)
    }
}
