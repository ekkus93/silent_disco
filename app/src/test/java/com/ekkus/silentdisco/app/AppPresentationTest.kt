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

    // Block 52.5: the core reaches CONNECTING (endpoint ready) *before* the
    // host replies, so a connecting listener is still asking for access and
    // must not be shown as approved.
    @Test
    fun connectingBeforeTheHostRepliesMapsToRequestingAccess() {
        val state = AppUiState(
            listenerState = ListenerLifecycleState.CONNECTING,
            connectionProgress = ConnectionProgressState(
                currentState = ListenerLifecycleState.CONNECTING,
                discovered = true,
                requested = true,
                approved = false,
            ),
        )

        assertThat(state.joinUiStep()).isEqualTo(JoinUiStep.REQUESTING_ACCESS)
    }

    @Test
    fun awaitingApprovalMapsToWaitingForHostApproval() {
        assertThat(
            AppUiState(listenerState = ListenerLifecycleState.AWAITING_APPROVAL).joinUiStep(),
        ).isEqualTo(JoinUiStep.WAITING_FOR_APPROVAL)
    }

    // Approved with no stream yet used to show "Connecting -- Creating the
    // nearby connection" with a spinner, which read as stuck.
    @Test
    fun approvedWithoutAStreamMapsToWaitingForPlayback() {
        val state = AppUiState(
            listenerState = ListenerLifecycleState.APPROVED,
            connectionProgress = ConnectionProgressState(
                currentState = ListenerLifecycleState.APPROVED,
                discovered = true,
                requested = true,
                connected = true,
                approved = true,
            ),
        )

        assertThat(state.joinUiStep()).isEqualTo(JoinUiStep.WAITING_FOR_PLAYBACK)
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

    @Test
    fun invalidInviteCodeProducesEditCodeAction() {
        val state = AppUiState(
            storageState = StorageInitializationState.READY,
            listenerState = ListenerLifecycleState.ERROR,
            lastError = "Incorrect invite code; join rejected",
        )

        val problem = state.derivedPersistentProblem()

        assertThat(problem?.kind).isEqualTo(UserProblemKind.INVALID_INVITE_CODE)
        assertThat(problem?.primaryAction).isEqualTo(UserProblemAction.EDIT_CODE)
        assertThat(problem?.secondaryAction).isEqualTo(UserProblemAction.RETURN_TO_SESSIONS)
    }

    @Test
    fun permissionFailureRoutesToSettings() {
        val state = AppUiState(
            storageState = StorageInitializationState.READY,
            listenerState = ListenerLifecycleState.ERROR,
            lastError = "Missing nearby connectivity permissions for discovery",
        )

        val problem = state.derivedPersistentProblem()

        assertThat(problem?.kind).isEqualTo(UserProblemKind.PERMISSION_REQUIRED)
        assertThat(problem?.primaryAction).isEqualTo(UserProblemAction.OPEN_SETTINGS)
    }

    @Test
    fun hostEndedSessionRoutesBackToSessions() {
        val state = AppUiState(
            storageState = StorageInitializationState.READY,
            listenerState = ListenerLifecycleState.DISCONNECTED,
            lastError = "Host ended the session",
        )

        val problem = state.derivedPersistentProblem()

        assertThat(problem?.kind).isEqualTo(UserProblemKind.HOST_ENDED)
        assertThat(problem?.primaryAction).isEqualTo(UserProblemAction.RETURN_TO_SESSIONS)
    }

    @Test
    fun transportFailureProvidesRetryAndReturnActions() {
        val state = AppUiState(
            storageState = StorageInitializationState.READY,
            listenerState = ListenerLifecycleState.ERROR,
            lastError = "Wi-Fi Direct transport connection failed",
        )

        val problem = state.derivedPersistentProblem()

        assertThat(problem?.kind).isEqualTo(UserProblemKind.TRANSPORT)
        assertThat(problem?.primaryAction).isEqualTo(UserProblemAction.RETRY)
        assertThat(problem?.secondaryAction).isEqualTo(UserProblemAction.RETURN_TO_SESSIONS)
    }
}
