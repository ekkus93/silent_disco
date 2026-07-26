package com.ekkus.silentdisco.app

import androidx.annotation.MainThread
import androidx.lifecycle.ViewModel
import com.ekkus.silentdisco.core.model.JoinRequest
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.receiveAsFlow

class WorkflowViewModel : ViewModel() {
    private val effectChannel = Channel<AppUiEffect>(capacity = Channel.BUFFERED)
    val effects: Flow<AppUiEffect> = effectChannel.receiveAsFlow()

    private var startupNavigationConsumed = false
    private var playbackNavigationConsumed = false
    private var cleared = false

    fun onUiStateChanged(state: AppUiState) {
        if (shouldNavigateHomeAfterStartup(state, startupNavigationConsumed)) {
            startupNavigationConsumed = true
            emit(AppUiEffect.NavigateHome)
        }

        if (state.selectedSession == null || state.listenerState in playbackResetStates) {
            playbackNavigationConsumed = false
        }
        if (shouldNavigateToListenerPlayback(state, playbackNavigationConsumed)) {
            playbackNavigationConsumed = true
            emit(AppUiEffect.NavigateListenerPlayback)
        }
    }

    fun onHostSessionCreationResult(started: Boolean) {
        if (started) emit(AppUiEffect.NavigateHostDashboard)
    }

    @MainThread
    fun handleJoinApproval(
        mainViewModel: MainViewModel,
        request: JoinRequest,
        action: JoinApprovalAction,
    ) {
        when (action) {
            JoinApprovalAction.REJECT -> mainViewModel.rejectJoinRequest(request)
            JoinApprovalAction.APPROVE_ONCE -> {
                mainViewModel.dispatchJoinApproval(request, rememberForFuture = false)
            }
            JoinApprovalAction.ALWAYS_ALLOW -> {
                mainViewModel.dispatchJoinApproval(request, rememberForFuture = true)
            }
        }
    }

    fun requestEndSessionConfirmation() {
        emit(AppUiEffect.ShowEndSessionConfirmation)
    }

    fun requestLeaveSessionConfirmation() {
        emit(AppUiEffect.ShowLeaveSessionConfirmation)
    }

    fun navigateHome() {
        emit(AppUiEffect.NavigateHome)
    }

    fun showTransientMessage(message: String) {
        require(message.isNotBlank()) { "Transient messages must not be blank" }
        emit(AppUiEffect.ShowTransientMessage(message))
    }

    private fun emit(effect: AppUiEffect) {
        val result = effectChannel.trySend(effect)
        check(result.isSuccess || cleared) {
            "Unable to enqueue workflow effect $effect"
        }
    }

    override fun onCleared() {
        cleared = true
        effectChannel.close()
        super.onCleared()
    }

    private companion object {
        val playbackResetStates = setOf(
            ListenerLifecycleState.IDLE,
            ListenerLifecycleState.SCANNING,
            ListenerLifecycleState.SESSION_SELECTED,
            ListenerLifecycleState.DISCONNECTED,
            ListenerLifecycleState.ERROR,
        )
    }
}
