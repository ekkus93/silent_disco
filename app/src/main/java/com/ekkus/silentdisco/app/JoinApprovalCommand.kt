package com.ekkus.silentdisco.app

import androidx.annotation.MainThread
import com.ekkus.silentdisco.core.model.JoinRequest

@MainThread
internal fun MainViewModel.dispatchJoinApproval(
    request: JoinRequest,
    rememberForFuture: Boolean,
) {
    withScopedApprovalLifetime(
        requestedLifetime = rememberForFuture,
        currentLifetime = { uiState.value.hostForm.rememberApprovedDevices },
        updateLifetime = { updateHostForm(rememberApprovedDevices = it) },
        dispatch = { approveJoinRequest(request) },
    )
}

internal inline fun withScopedApprovalLifetime(
    requestedLifetime: Boolean,
    currentLifetime: () -> Boolean,
    updateLifetime: (Boolean) -> Unit,
    dispatch: () -> Unit,
) {
    val previousLifetime = currentLifetime()
    updateLifetime(requestedLifetime)
    try {
        dispatch()
    } finally {
        updateLifetime(previousLifetime)
    }
}
