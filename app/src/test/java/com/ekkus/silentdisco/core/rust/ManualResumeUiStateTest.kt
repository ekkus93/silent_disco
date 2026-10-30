package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.model.PlaybackState
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class ManualResumeUiStateTest {
    @Test
    fun sameStreamResumeWithLockedClockLeavesPausedForPlayingWithoutReopeningOutput() {
        val resumed = manualResumedUiState(trustedForFuture = false, syncLocked = true)
        assertThat(resumed.playbackState).isEqualTo(PlaybackState.PLAYING)
    }

    @Test
    fun sameStreamResumeWithoutClockLockWaitsForRustSyncRatherThanClaimingPlayback() {
        val resumed = manualResumedUiState(trustedForFuture = true, syncLocked = false)
        assertThat(resumed.playbackState).isEqualTo(PlaybackState.BUFFERING)
    }
}
