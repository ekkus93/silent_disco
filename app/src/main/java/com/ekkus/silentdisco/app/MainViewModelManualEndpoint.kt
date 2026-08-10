package com.ekkus.silentdisco.app

import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.core.model.ManualConnectUiState
import com.ekkus.silentdisco.core.model.canConnect
import com.ekkus.silentdisco.core.rust.ManualEndpointParseResult
import com.ekkus.silentdisco.core.rust.P2ValidatedInvitation
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

/**
 * Pre-fills the manual-endpoint form from a verified QR invitation's
 * embedded connection payload (Block 31 desktop-QR support) -- the exact
 * same JSON shape and parser [updateManualEndpointInputImpl] already
 * accepts from a pasted payload, so this reuses that whole path rather than
 * connecting directly here. Deliberately does not auto-connect: the form's
 * own async parse still has to populate its preview fields before
 * [ManualConnectUiState.canConnect] allows it, and letting the user see and
 * confirm what they are about to join (host, session, ports) before
 * connecting matches this app's "no silent auto-admit" posture even though
 * the payload itself is already signature-verified.
 *
 * No-ops if the invitation carries no connection payload (Android's own
 * peer-to-peer QR flow) -- callers are expected to gate on that themselves,
 * but this stays defensive rather than silently misbehaving if they don't.
 */
internal fun MainViewModel.prefillManualEndpointFromInvitationImpl(invitation: P2ValidatedInvitation) {
    val connectionPayloadJson = invitation.connectionPayloadJson ?: return
    updateManualEndpointInputImpl(connectionPayloadJson)
    updateManualEndpointInviteCodeImpl(invitation.inviteCode.orEmpty())
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
