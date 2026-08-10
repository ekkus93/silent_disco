package com.ekkus.silentdisco.app

import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.BuildConfig
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

internal fun MainViewModel.scanForSessionsImpl() {
    if (!requirePersistenceReady("scan for sessions")) return
    logger.i("listener.scan", "Scanning for nearby sessions")
    scanJob?.cancel()
    _uiState.value = _uiState.value.copy(lastMessage = "Scanning for nearby sessions…", lastError = null)
    ensureRustListenerCore().startDiscovery()

    scanJob = viewModelScope.launch {
        val scanWindowMs = _uiState.value.tuningSettings.normalized().scanWindowMs
        delay(scanWindowMs)
        refreshDiscoveredSessions()
    }
}

internal fun MainViewModel.requestJoinImpl() {
    if (!requirePersistenceReady("join a session")) return
    val session = _uiState.value.selectedSession ?: run {
        _uiState.value = _uiState.value.copy(lastError = "Select a session before joining")
        return
    }
    if (_uiState.value.discoveredSessions.none { it.id == session.id }) {
        _uiState.value = _uiState.value.copy(lastError = "Selected session disappeared before join")
        diagnosticsStore.updateListener { it.copy(lastError = "Selected session disappeared before join") }
        refreshListenerDiagnostics()
        return
    }
    val inviteCode = _uiState.value.connectionProgress.inviteCode.ifBlank { null }
    if (session.inviteCodeRequired && inviteCode == null) {
        _uiState.value = _uiState.value.copy(lastError = "Invite code required")
        return
    }
    logger.i("listener.join", "Join request created for ${session.id}")
    _uiState.value = _uiState.value.copy(lastMessage = "Connecting to host", lastError = null)
    val shouldSimulate = BuildConfig.DEBUG && session.id.startsWith(DEMO_SESSION_ID_PREFIX)
    if (shouldSimulate) {
        val shouldReject = session.inviteCodeRequired && inviteCode != "1234"
        simulateApprovalAndPlayback(session.id, shouldReject)
        return
    }
    ensureRustListenerCore().submitJoin(inviteCode)
}
