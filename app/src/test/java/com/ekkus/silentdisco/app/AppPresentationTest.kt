package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class AppPresentationTest {
    @Test
    fun everyListenerLifecycleStateMapsToAJoinStep() {
        ListenerLifecycleState.entries.forEach { lifecycleState ->
            val step = AppUiState(listenerState = lifecycleState).joinUiStep()
            assertThat(step).isNotNull()
        }
    }

    @Test
    fun listenerPlayingMapsToCompleteOnlyWhenLifecycleIsPlaying() {
        assertThat(
            AppUiState(listenerState = ListenerLifecycleState.PLAYING).joinUiStep(),
        ).isEqualTo(JoinUiStep.COMPLETE)
        assertThat(
            AppUiState(listenerState = ListenerLifecycleState.BUFFERING).joinUiStep(),
        ).isEqualTo(JoinUiStep.SYNCING_AUDIO)
    }

    @Test
    fun hostErrorCanNeverProduceGoodHealth() {
        val state = AppUiState(
            hostState = HostLifecycleState.ERROR,
            hostPlaybackState = PlaybackState.ERROR,
        )

        assertThat(state.hostSessionHealthSummary().level).isEqualTo(SessionHealthLevel.CRITICAL)
    }

    @Test
    fun desyncedListenersProduceAttentionHealth() {
        val baseline = AppUiState(hostState = HostLifecycleState.STREAMING)
        val state = baseline.copy(
            hostDiagnostics = baseline.hostDiagnostics.copy(desyncedListenerCount = 2),
        )

        val summary = state.hostSessionHealthSummary()

        assertThat(summary.level).isEqualTo(SessionHealthLevel.ATTENTION)
        assertThat(summary.affectedListenerCount).isEqualTo(2)
    }

    @Test
    fun healthyPlayingListenerProducesGoodHealth() {
        val baseline = AppUiState(
            listenerState = ListenerLifecycleState.PLAYING,
            listenerPlaybackState = PlaybackState.PLAYING,
        )
        val state = baseline.copy(
            listenerSyncState = baseline.listenerSyncState.copy(
                confidence = SyncQualityBadge.GOOD,
            ),
        )

        assertThat(state.listenerConnectionHealthSummary().level).isEqualTo(SessionHealthLevel.GOOD)
    }

    @Test
    fun storageFailureProducesPersistentProblem() {
        val state = AppUiState(
            storageState = StorageInitializationState.FATAL_FAILURE,
            storageError = "checksum mismatch",
        )

        val problem = state.derivedPersistentProblem()

        assertThat(problem?.kind).isEqualTo(UserProblemKind.STORAGE_FATAL)
        assertThat(problem?.technicalDetail).contains("checksum mismatch")
    }
}
