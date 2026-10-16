package com.ekkus.silentdisco.app

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.test.runTest
import org.junit.Test

class HostPacketPresentationTailTest {
    @Test
    fun failureStopWaitsForTheLastSentPacketsFullPresentationInterval() = runTest {
        val endTimes = mutableListOf<Long>()
        var nowMs = 1_250L

        val deadline = awaitHostPacketPresentationTail(
            packetPresentationTimeMs = 1_000L,
            presentationOffsetMs = 100L,
            packetDurationMs = 200L,
            elapsedRealtimeMs = { nowMs },
            wait = { remaining ->
                endTimes += nowMs
                nowMs += remaining
            },
        )
        endTimes += deadline

        assertThat(endTimes).containsExactly(1_250L, 1_300L).inOrder()
        assertThat(nowMs).isEqualTo(1_300L)
    }

    @Test
    fun packetAlreadyPresentedDoesNotWaitOrMoveTheStopDeadline() = runTest {
        var waits = 0
        val deadline = awaitHostPacketPresentationTail(
            packetPresentationTimeMs = 1_000L,
            presentationOffsetMs = 100L,
            packetDurationMs = 200L,
            elapsedRealtimeMs = { 1_500L },
            wait = { waits += 1 },
        )

        assertThat(deadline).isEqualTo(1_300L)
        assertThat(waits).isEqualTo(0)
    }
}
