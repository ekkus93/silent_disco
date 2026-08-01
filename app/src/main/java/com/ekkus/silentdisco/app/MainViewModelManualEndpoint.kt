package com.ekkus.silentdisco.app

import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.core.model.ManualConnectUiState
import com.ekkus.silentdisco.core.model.canConnect
import com.ekkus.silentdisco.core.rust.ManualEndpointParseResult
import kotlinx.coroutines.launch

internal fun MainViewModel.observeManualEndpointConnection() {
    viewModelScope.launch {
        manualListenerController.connectState.collect { state ->
            _uiState.value = _uiState.value.copy(
                manualConnectState = state,
                lastError = when (state) {
                    is ManualConnectUiState.Rejected -> "Host rejected the connection: ${state.reason}"
                    is ManualConnectUiState.Disconnected ->
                        state.message?.let { "Host disconnected: $it" } ?: "Host disconnected"
                    is ManualConnectUiState.Failed -> state.message
                    else -> _uiState.value.lastError
                },
                lastMessage = if (state is ManualConnectUiState.Approved) {
                    "Manual connection approved"
                } else {
                    _uiState.value.lastMessage
                },
            )
        }
    }
}

internal fun MainViewModel.updateManualEndpointInputImpl(raw: String) {
    _uiState.value = _uiState.value.copy(
        manualEndpointForm = _uiState.value.manualEndpointForm.copy(rawInput = raw),
    )
    if (raw.isBlank()) {
        _uiState.value = _uiState.value.copy(
            manualEndpointForm = _uiState.value.manualEndpointForm.copy(
                validationError = null,
                hostAddress = null,
                sessionId = null,
                protocolVersion = null,
                inviteCodeRequired = false,
            ),
        )
        return
    }
    viewModelScope.launch {
        when (val result = manualListenerController.parse(raw)) {
            is ManualEndpointParseResult.Valid -> {
                if (_uiState.value.manualEndpointForm.rawInput != raw) return@launch
                _uiState.value = _uiState.value.copy(
                    manualEndpointForm = _uiState.value.manualEndpointForm.copy(
                        validationError = null,
                        hostAddress = result.endpoint.hostAddress,
                        sessionId = result.endpoint.sessionId,
                        protocolVersion = result.endpoint.protocolVersion.toInt(),
                        inviteCodeRequired = result.endpoint.inviteCodeRequired,
                    ),
                )
            }
            is ManualEndpointParseResult.Invalid -> {
                if (_uiState.value.manualEndpointForm.rawInput != raw) return@launch
                _uiState.value = _uiState.value.copy(
                    manualEndpointForm = _uiState.value.manualEndpointForm.copy(
                        validationError = result.message,
                        hostAddress = null,
                        sessionId = null,
                        protocolVersion = null,
                        inviteCodeRequired = false,
                    ),
                )
            }
        }
    }
}

internal fun MainViewModel.updateManualEndpointInviteCodeImpl(code: String) {
    _uiState.value = _uiState.value.copy(
        manualEndpointForm = _uiState.value.manualEndpointForm.copy(inviteCode = code),
    )
}

internal fun MainViewModel.connectManualEndpointImpl() {
    val form = _uiState.value.manualEndpointForm
    if (!form.canConnect()) {
        _uiState.value = _uiState.value.copy(
            lastError = form.validationError ?: "Enter a valid connection payload",
        )
        return
    }
    logger.i("listener.manual_connect", "Connecting to manual endpoint ${form.sessionId}")
    _uiState.value = _uiState.value.copy(lastMessage = "Connecting to host", lastError = null)
    viewModelScope.launch {
        manualListenerController.connect(
            scope = viewModelScope,
            rawInput = form.rawInput,
            localDeviceId = localListenerDeviceId,
            displayName = "This Android Listener",
            inviteCode = form.inviteCode.ifBlank { null },
        )
    }
}

internal fun MainViewModel.cancelManualEndpointConnectImpl() {
    viewModelScope.launch {
        manualListenerController.disconnect("listener_cancelled")
        manualListenerController.reset()
        _uiState.value = _uiState.value.copy(
            manualConnectState = ManualConnectUiState.Idle,
            lastMessage = "Manual connection cancelled",
        )
    }
}
