package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState

sealed interface AppUiEffect {
    data object NavigateHome : AppUiEffect
    data object NavigateHostDashboard : AppUiEffect
    data object NavigateListenerPlayback : AppUiEffect
    data object ShowEndSessionConfirmation : AppUiEffect
    data object ShowLeaveSessionConfirmation : AppUiEffect
    data class ShowTransientMessage(val message: String) : AppUiEffect
}

internal fun shouldNavigateHomeAfterStartup(
    state: AppUiState,
    alreadyConsumed: Boolean,
): Boolean = state.storageState == StorageInitializationState.READY && !alreadyConsumed

internal fun shouldNavigateToListenerPlayback(
    state: AppUiState,
    alreadyConsumed: Boolean,
): Boolean = state.listenerState == ListenerLifecycleState.PLAYING &&
    state.listenerPlaybackState == PlaybackState.PLAYING &&
    !alreadyConsumed
