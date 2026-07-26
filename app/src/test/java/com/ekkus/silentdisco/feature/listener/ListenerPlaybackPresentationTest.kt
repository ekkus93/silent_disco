package com.ekkus.silentdisco.feature.listener

import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.ui.components.StatusTone
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class ListenerPlaybackPresentationTest {
    @Test
    fun healthyPlaybackIsPositive() {
        val state = AppUiState(
            listenerState = ListenerLifecycleState.PLAYING,
            listenerPlaybackState = PlaybackState.PLAYING,
        )

        assertThat(listenerPlaybackStatus(state)).isEqualTo("Playing in sync")
        assertThat(listenerPlaybackTone(state)).isEqualTo(StatusTone.POSITIVE)
    }

    @Test
    fun bufferingAndReconnectAreInProgress() {
        val buffering = AppUiState(
            listenerState = ListenerLifecycleState.BUFFERING,
            listenerPlaybackState = PlaybackState.BUFFERING,
        )
        val reconnecting = AppUiState(
            listenerState = ListenerLifecycleState.RECONNECTING,
            listenerPlaybackState = PlaybackState.PLAYING,
        )

        assertThat(listenerPlaybackStatus(buffering)).isEqualTo("Buffering")
        assertThat(listenerPlaybackTone(buffering)).isEqualTo(StatusTone.IN_PROGRESS)
        assertThat(listenerPlaybackStatus(reconnecting)).isEqualTo("Reconnecting")
        assertThat(listenerPlaybackTone(reconnecting)).isEqualTo(StatusTone.IN_PROGRESS)
    }

    @Test
    fun disconnectAndPlaybackFailureAreCritical() {
        val disconnected = AppUiState(
            listenerState = ListenerLifecycleState.DISCONNECTED,
            listenerPlaybackState = PlaybackState.STOPPED,
        )
        val playbackError = AppUiState(
            listenerState = ListenerLifecycleState.PLAYING,
            listenerPlaybackState = PlaybackState.ERROR,
        )

        assertThat(listenerPlaybackStatus(disconnected)).isEqualTo("Connection lost")
        assertThat(listenerPlaybackTone(disconnected)).isEqualTo(StatusTone.CRITICAL)
        assertThat(listenerPlaybackStatus(playbackError)).isEqualTo("Playback problem")
        assertThat(listenerPlaybackTone(playbackError)).isEqualTo(StatusTone.CRITICAL)
    }

    @Test
    fun desynchronizedPlaybackIsAttentionNotHealthy() {
        val state = AppUiState(
            listenerState = ListenerLifecycleState.DESYNCED,
            listenerPlaybackState = PlaybackState.PLAYING,
        )

        assertThat(listenerPlaybackStatus(state)).isEqualTo("Audio is out of sync")
        assertThat(listenerPlaybackTone(state)).isEqualTo(StatusTone.ATTENTION)
    }
}
