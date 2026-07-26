package com.ekkus.silentdisco.app

sealed interface AppUiEffect {
    data object NavigateHome : AppUiEffect
    data object NavigateHostDashboard : AppUiEffect
    data object NavigateListenerPlayback : AppUiEffect
    data object ShowEndSessionConfirmation : AppUiEffect
    data object ShowLeaveSessionConfirmation : AppUiEffect
    data class ShowTransientMessage(val message: String) : AppUiEffect
}
