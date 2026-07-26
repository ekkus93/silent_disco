package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class AppUiEffectTest {
    @Test
    fun startupNavigationEmitsOnlyWhenReadyAndUnconsumed() {
        val ready = AppUiState(storageState = StorageInitializationState.READY)
        val loading = AppUiState(storageState = StorageInitializationState.INITIALIZING)

        assertThat(shouldNavigateHomeAfterStartup(ready, alreadyConsumed = false)).isTrue()
        assertThat(shouldNavigateHomeAfterStartup(ready, alreadyConsumed = true)).isFalse()
        assertThat(shouldNavigateHomeAfterStartup(loading, alreadyConsumed = false)).isFalse()
    }

    @Test
    fun playbackNavigationRequiresActualPlayingStateAndIsOneShot() {
        val playing = AppUiState(
            listenerState = ListenerLifecycleState.PLAYING,
            listenerPlaybackState = PlaybackState.PLAYING,
        )
        val buffering = playing.copy(
            listenerState = ListenerLifecycleState.BUFFERING,
            listenerPlaybackState = PlaybackState.BUFFERING,
        )

        assertThat(shouldNavigateToListenerPlayback(playing, alreadyConsumed = false)).isTrue()
        assertThat(shouldNavigateToListenerPlayback(playing, alreadyConsumed = true)).isFalse()
        assertThat(shouldNavigateToListenerPlayback(buffering, alreadyConsumed = false)).isFalse()
    }
}
