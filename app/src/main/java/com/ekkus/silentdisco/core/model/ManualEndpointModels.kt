package com.ekkus.silentdisco.core.model

/** Local editing state for the manual desktop-endpoint paste/input form. */
data class ManualEndpointFormState(
    val rawInput: String = "",
    val inviteCode: String = "",
    val validationError: String? = null,
    val hostAddress: String? = null,
    val sessionId: String? = null,
    val protocolVersion: Int? = null,
    val inviteCodeRequired: Boolean = false,
)

/** Lifecycle of one manual-endpoint connection attempt, driven by real Rust transport events. */
sealed interface ManualConnectUiState {
    data object Idle : ManualConnectUiState

    data object Connecting : ManualConnectUiState

    data class AwaitingApproval(val hostName: String, val sessionName: String) : ManualConnectUiState

    data class Approved(val trustedForFuture: Boolean) : ManualConnectUiState

    /** A stream is actually flowing from the host; [playbackState] reflects real local playback. */
    data class Streaming(val trustedForFuture: Boolean, val playbackState: PlaybackState) : ManualConnectUiState

    data class Rejected(val reason: String) : ManualConnectUiState

    data class Disconnected(val message: String?) : ManualConnectUiState

    data class Failed(val message: String) : ManualConnectUiState
}

fun ManualEndpointFormState.canConnect(): Boolean =
    validationError == null &&
        hostAddress != null &&
        (!inviteCodeRequired || inviteCode.isNotBlank())

fun ManualConnectUiState.isInProgress(): Boolean = this is ManualConnectUiState.Connecting ||
    this is ManualConnectUiState.AwaitingApproval
