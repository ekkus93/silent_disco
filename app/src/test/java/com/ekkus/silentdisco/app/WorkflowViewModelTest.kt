package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SessionInfo
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.runTest
import org.junit.Test

class WorkflowViewModelTest {
    @Test
    fun startupNavigationIsEmittedOnlyOnce() = runTest {
        val viewModel = WorkflowViewModel()
        val ready = AppUiState(storageState = StorageInitializationState.READY)

        viewModel.onUiStateChanged(ready)
        assertThat(viewModel.effects.first()).isEqualTo(AppUiEffect.NavigateHome)

        viewModel.onUiStateChanged(ready)
        viewModel.onHostSessionCreationResult(started = true)
        assertThat(viewModel.effects.first()).isEqualTo(AppUiEffect.NavigateHostDashboard)
    }

    @Test
    fun listenerPlaybackNavigationIsSingleShotUntilWorkflowResets() = runTest {
        val viewModel = WorkflowViewModel()
        val session = SessionInfo(
            id = "session-workflow",
            name = "Workflow test",
            hostDeviceName = "Host phone",
            approvalMode = ApprovalMode.MANUAL,
            inviteCodeRequired = false,
        )
        val playing = AppUiState(
            storageState = StorageInitializationState.INITIALIZING,
            selectedSession = session,
            listenerState = ListenerLifecycleState.PLAYING,
            listenerPlaybackState = PlaybackState.PLAYING,
        )

        viewModel.onUiStateChanged(playing)
        assertThat(viewModel.effects.first()).isEqualTo(AppUiEffect.NavigateListenerPlayback)

        viewModel.onUiStateChanged(playing)
        viewModel.requestEndSessionConfirmation()
        assertThat(viewModel.effects.first()).isEqualTo(AppUiEffect.ShowEndSessionConfirmation)

        viewModel.onUiStateChanged(
            AppUiState(
                storageState = StorageInitializationState.INITIALIZING,
                listenerState = ListenerLifecycleState.IDLE,
            ),
        )
        viewModel.onUiStateChanged(playing)
        assertThat(viewModel.effects.first()).isEqualTo(AppUiEffect.NavigateListenerPlayback)
    }

    @Test
    fun confirmationAndTransientEffectsPreserveRequestOrder() = runTest {
        val viewModel = WorkflowViewModel()

        viewModel.requestLeaveSessionConfirmation()
        viewModel.showTransientMessage("Invite code copied")

        assertThat(viewModel.effects.first()).isEqualTo(AppUiEffect.ShowLeaveSessionConfirmation)
        assertThat(viewModel.effects.first()).isEqualTo(AppUiEffect.ShowTransientMessage("Invite code copied"))
    }
}
