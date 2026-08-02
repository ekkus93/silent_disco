package com.ekkus.silentdisco.feature.listener

import com.ekkus.silentdisco.core.model.PlaybackState
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class ManualEndpointScreenPresentationTest {
    @Test
    fun everyPlaybackStateHasAVisibleLabel() {
        assertThat(playbackStateLabel(PlaybackState.BUFFERING)).isEqualTo("Buffering…")
        assertThat(playbackStateLabel(PlaybackState.PLAYING)).isEqualTo("Playing")
        assertThat(playbackStateLabel(PlaybackState.PAUSED)).isEqualTo("Paused by the host")
        assertThat(playbackStateLabel(PlaybackState.UNDERRUN)).contains("recovering")
        assertThat(playbackStateLabel(PlaybackState.STOPPED)).isEqualTo("Stopped")
    }
}
