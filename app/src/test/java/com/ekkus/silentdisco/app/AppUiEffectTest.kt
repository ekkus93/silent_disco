package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.model.SyncState
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
            selectedSession = sessionForPlayback(),
            connectionProgress = ConnectionProgressState(
                approved = true,
                connected = true,
                synced = true,
                buffered = true,
                playing = true,
            ),
            listenerSyncState = SyncState(confidence = SyncQualityBadge.GOOD),
        )
        val buffering = playing.copy(
            listenerState = ListenerLifecycleState.BUFFERING,
            listenerPlaybackState = PlaybackState.BUFFERING,
        )

        assertThat(shouldNavigateToListenerPlayback(playing, alreadyConsumed = false)).isTrue()
        assertThat(shouldNavigateToListenerPlayback(playing, alreadyConsumed = true)).isFalse()
        assertThat(shouldNavigateToListenerPlayback(buffering, alreadyConsumed = false)).isFalse()
        assertThat(
            shouldNavigateToListenerPlayback(
                playing.copy(connectionProgress = playing.connectionProgress.copy(synced = false)),
                alreadyConsumed = false,
            ),
        ).isFalse()
        assertThat(
            shouldNavigateToListenerPlayback(
                playing.copy(connectionProgress = playing.connectionProgress.copy(approved = false)),
                alreadyConsumed = false,
            ),
        ).isFalse()
        assertThat(
            shouldNavigateToListenerPlayback(
                playing.copy(connectionProgress = playing.connectionProgress.copy(buffered = false)),
                alreadyConsumed = false,
            ),
        ).isFalse()
    }

    private fun sessionForPlayback() = SessionInfo(
        id = "session-playable",
        name = "Playable session",
        hostDeviceName = "Host phone",
        approvalMode = ApprovalMode.MANUAL,
        inviteCodeRequired = false,
    )
}
