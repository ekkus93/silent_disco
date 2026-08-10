package com.ekkus.silentdisco.app

import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.TransportConnectionState
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

    internal fun MainViewModel.simulateApprovalAndPlayback(sessionId: String, reject: Boolean) {
        viewModelScope.launch {
            delay(500)
            if (reject) {
                _uiState.value = _uiState.value.copy(
                    listenerState = ListenerLifecycleState.ERROR,
                    lastError = "Host rejected join. Check the invite code and try again.",
                )
                diagnosticsStore.updateListener {
                    it.copy(sessionId = sessionId, lastError = "Join rejected by host")
                }
                refreshListenerDiagnostics()
                return@launch
            }
            _uiState.value = _uiState.value.copy(
                listenerState = ListenerLifecycleState.APPROVED,
                connectionProgress = _uiState.value.connectionProgress.copy(
                    currentState = ListenerLifecycleState.APPROVED,
                    approved = true,
                ),
            )
            delay(400)
            wifiDirectService.connectToSession(_uiState.value.selectedSession ?: demoSessions().first())
            if (wifiDirectService.snapshot.value.state != TransportConnectionState.CONNECTED) {
                handleListenerConnectionFailure("Failed to connect to host transport")
                return@launch
            }
            _uiState.value = _uiState.value.copy(
                listenerState = ListenerLifecycleState.CONNECTING,
                connectionProgress = _uiState.value.connectionProgress.copy(
                    currentState = ListenerLifecycleState.CONNECTING,
                    connected = true,
                ),
            )
            delay(400)
            requestListenerSyncProbe(source = "Demo clock sync")
            _uiState.value = _uiState.value.copy(
                listenerState = ListenerLifecycleState.BUFFERING,
                connectionProgress = _uiState.value.connectionProgress.copy(
                    currentState = ListenerLifecycleState.BUFFERING,
                    synced = true,
                    buffered = false,
                    playing = false,
                ),
            )
            startListenerPlaybackSimulation(sessionId)
        }
    }
