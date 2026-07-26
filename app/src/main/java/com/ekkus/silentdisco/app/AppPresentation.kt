package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SyncQualityBadge

enum class HostSetupStep {
    MUSIC,
    ACCESS,
}

enum class JoinUiStep {
    FINDING_HOST,
    REQUESTING_ACCESS,
    WAITING_FOR_APPROVAL,
    CONNECTING,
    SYNCING_AUDIO,
    COMPLETE,
}

enum class SessionHealthLevel {
    GOOD,
    ATTENTION,
    CRITICAL,
    UNKNOWN,
}

data class SessionHealthSummary(
    val level: SessionHealthLevel,
    val title: String,
    val detail: String,
    val affectedListenerCount: Int = 0,
)

enum class UserProblemKind {
    STORAGE_RECOVERABLE,
    STORAGE_FATAL,
    PERMISSION_REQUIRED,
    HOST_SESSION,
    JOIN_REJECTED,
    INVALID_INVITE_CODE,
    HOST_ENDED,
    HOST_UNREACHABLE,
    TRANSPORT,
    CONNECTION_LOST,
    SYNCHRONIZATION,
    PLAYBACK,
    PERSISTENCE,
    UNKNOWN,
}

enum class UserProblemAction {
    RETRY,
    EDIT_CODE,
    RETURN_TO_SESSIONS,
    OPEN_SETTINGS,
    RESYNCHRONIZE,
    RECONNECT,
    SHARE_SUPPORT_REPORT,
    DISMISS,
}

data class UserFacingProblem(
    val id: String,
    val kind: UserProblemKind,
    val title: String,
    val detail: String,
    val primaryAction: UserProblemAction? = null,
    val secondaryAction: UserProblemAction? = null,
    val technicalDetail: String? = null,
    val dismissible: Boolean = false,
)

enum class JoinApprovalAction {
    APPROVE_ONCE,
    ALWAYS_ALLOW,
    REJECT,
}

fun UserProblemAction.label(): String = when (this) {
    UserProblemAction.RETRY -> "Retry"
    UserProblemAction.EDIT_CODE -> "Edit invite code"
    UserProblemAction.RETURN_TO_SESSIONS -> "Return to sessions"
    UserProblemAction.OPEN_SETTINGS -> "Open Settings"
    UserProblemAction.RESYNCHRONIZE -> "Resynchronize audio"
    UserProblemAction.RECONNECT -> "Reconnect"
    UserProblemAction.SHARE_SUPPORT_REPORT -> "Share support report"
    UserProblemAction.DISMISS -> "Dismiss"
}

/**
 * Maps domain progress to the smaller set of user-facing join steps.
 *
 * Progress flags take precedence over the coarse lifecycle enum because the
 * transport can enter CONNECTING before host approval has been received.
 */
fun AppUiState.joinUiStep(): JoinUiStep = when {
    listenerState == ListenerLifecycleState.PLAYING || connectionProgress.playing ->
        JoinUiStep.COMPLETE

    listenerState in setOf(
        ListenerLifecycleState.SYNCING_CLOCK,
        ListenerLifecycleState.BUFFERING,
        ListenerLifecycleState.DESYNCED,
    ) || connectionProgress.synced || connectionProgress.buffered ->
        JoinUiStep.SYNCING_AUDIO

    connectionProgress.approved || listenerState == ListenerLifecycleState.APPROVED ->
        JoinUiStep.CONNECTING

    connectionProgress.requested && !connectionProgress.approved ->
        JoinUiStep.WAITING_FOR_APPROVAL

    listenerState == ListenerLifecycleState.JOIN_REQUESTED ->
        JoinUiStep.REQUESTING_ACCESS

    listenerState in setOf(
        ListenerLifecycleState.CONNECTING,
        ListenerLifecycleState.RECONNECTING,
        ListenerLifecycleState.DISCONNECTED,
        ListenerLifecycleState.ERROR,
    ) -> JoinUiStep.CONNECTING

    else -> JoinUiStep.FINDING_HOST
}

fun AppUiState.hostSessionHealthSummary(): SessionHealthSummary {
    val affectedListeners = hostDiagnostics.desyncedListenerCount
    if (
        hostState == HostLifecycleState.ERROR ||
        hostPlaybackState == PlaybackState.ERROR ||
        hostDiagnostics.streamState == PlaybackState.ERROR ||
        !hostDiagnostics.lastError.isNullOrBlank()
    ) {
        return SessionHealthSummary(
            level = SessionHealthLevel.CRITICAL,
            title = "Session needs attention",
            detail = "Playback or the host connection reported a problem.",
            affectedListenerCount = affectedListeners,
        )
    }

    if (affectedListeners > 0) {
        return SessionHealthSummary(
            level = SessionHealthLevel.ATTENTION,
            title = "Some listeners need help",
            detail = "$affectedListeners listener(s) are out of sync.",
            affectedListenerCount = affectedListeners,
        )
    }

    if (
        hostState == HostLifecycleState.STREAMING ||
        hostState == HostLifecycleState.PAUSED ||
        hostState == HostLifecycleState.READY ||
        hostState == HostLifecycleState.WAITING_FOR_LISTENERS
    ) {
        val connected = hostDiagnostics.connectedListenerCount
        return SessionHealthSummary(
            level = SessionHealthLevel.GOOD,
            title = "Session health is good",
            detail = if (connected == 0) {
                "The session is ready for listeners."
            } else {
                "All $connected connected listener(s) are synchronized."
            },
        )
    }

    return SessionHealthSummary(
        level = SessionHealthLevel.UNKNOWN,
        title = "Session status is not available yet",
        detail = "Start the session to see connection health.",
    )
}

fun AppUiState.listenerConnectionHealthSummary(): SessionHealthSummary {
    if (
        listenerState == ListenerLifecycleState.ERROR ||
        listenerPlaybackState == PlaybackState.ERROR
    ) {
        return SessionHealthSummary(
            level = SessionHealthLevel.CRITICAL,
            title = "Playback or connection problem",
            detail = "Silent Disco could not continue the listener workflow.",
            affectedListenerCount = 1,
        )
    }

    if (listenerState == ListenerLifecycleState.DISCONNECTED) {
        return SessionHealthSummary(
            level = SessionHealthLevel.CRITICAL,
            title = "Connection lost",
            detail = "Reconnect to continue listening.",
            affectedListenerCount = 1,
        )
    }

    if (listenerState == ListenerLifecycleState.DESYNCED) {
        return SessionHealthSummary(
            level = SessionHealthLevel.ATTENTION,
            title = "Audio is out of sync",
            detail = "Resynchronize audio to catch up with the host.",
            affectedListenerCount = 1,
        )
    }

    if (listenerState == ListenerLifecycleState.RECONNECTING) {
        return SessionHealthSummary(
            level = SessionHealthLevel.ATTENTION,
            title = "Connection is recovering",
            detail = "Silent Disco is trying to reconnect to the host.",
            affectedListenerCount = 1,
        )
    }

    if (
        listenerState == ListenerLifecycleState.PLAYING &&
        listenerPlaybackState == PlaybackState.PLAYING &&
        listenerSyncState.confidence in setOf(SyncQualityBadge.EXCELLENT, SyncQualityBadge.GOOD)
    ) {
        return SessionHealthSummary(
            level = SessionHealthLevel.GOOD,
            title = "Playing in sync",
            detail = "The connection and audio synchronization are stable.",
        )
    }

    return SessionHealthSummary(
        level = SessionHealthLevel.UNKNOWN,
        title = "Connection status is updating",
        detail = "Silent Disco is monitoring the host connection.",
    )
}

private fun String?.normalizedProblemText(): String = orEmpty().lowercase()

private fun AppUiState.listenerErrorProblem(): UserFacingProblem {
    val errorText = lastError.normalizedProblemText()
    return when {
        "permission" in errorText -> UserFacingProblem(
            id = "listener-permission",
            kind = UserProblemKind.PERMISSION_REQUIRED,
            title = "Nearby-device access is required",
            detail = "Allow nearby-device access in Settings, then try again.",
            primaryAction = UserProblemAction.OPEN_SETTINGS,
            secondaryAction = UserProblemAction.RETURN_TO_SESSIONS,
            technicalDetail = lastError,
        )

        "invite code" in errorText || "incorrect code" in errorText -> UserFacingProblem(
            id = "invalid-invite-code",
            kind = UserProblemKind.INVALID_INVITE_CODE,
            title = "The invite code was not accepted",
            detail = "Check the code with the host and enter it again.",
            primaryAction = UserProblemAction.EDIT_CODE,
            secondaryAction = UserProblemAction.RETURN_TO_SESSIONS,
            technicalDetail = lastError,
        )

        "reject" in errorText || "did not approve" in errorText -> UserFacingProblem(
            id = "join-rejected",
            kind = UserProblemKind.JOIN_REJECTED,
            title = "The host did not approve this request",
            detail = "Ask the host to approve this phone or choose another session.",
            primaryAction = UserProblemAction.RETURN_TO_SESSIONS,
            secondaryAction = UserProblemAction.RETRY,
            technicalDetail = lastError,
        )

        "host ended" in errorText || "session ended" in errorText -> UserFacingProblem(
            id = "host-ended-session",
            kind = UserProblemKind.HOST_ENDED,
            title = "The host ended the session",
            detail = "Return to nearby sessions to look for another host.",
            primaryAction = UserProblemAction.RETURN_TO_SESSIONS,
            technicalDetail = lastError,
        )

        "sync" in errorText || "clock" in errorText -> UserFacingProblem(
            id = "listener-sync-failure",
            kind = UserProblemKind.SYNCHRONIZATION,
            title = "Audio could not be synchronized",
            detail = "Retry the connection or return to nearby sessions.",
            primaryAction = UserProblemAction.RETRY,
            secondaryAction = UserProblemAction.RETURN_TO_SESSIONS,
            technicalDetail = lastError,
        )

        "playback" in errorText || "audio output" in errorText || "audio track" in errorText ->
            UserFacingProblem(
                id = "listener-playback",
                kind = UserProblemKind.PLAYBACK,
                title = "Playback could not start",
                detail = "Retry the session. A support report may help if the problem continues.",
                primaryAction = UserProblemAction.RETRY,
                secondaryAction = UserProblemAction.SHARE_SUPPORT_REPORT,
                technicalDetail = lastError,
            )

        "transport" in errorText || "wi-fi" in errorText || "bluetooth" in errorText ->
            UserFacingProblem(
                id = "listener-transport",
                kind = UserProblemKind.TRANSPORT,
                title = "The nearby connection failed",
                detail = "Move closer to the host and retry the connection.",
                primaryAction = UserProblemAction.RETRY,
                secondaryAction = UserProblemAction.RETURN_TO_SESSIONS,
                technicalDetail = lastError,
            )

        "disappeared" in errorText || "unreachable" in errorText || "out of range" in errorText ->
            UserFacingProblem(
                id = "host-unreachable",
                kind = UserProblemKind.HOST_UNREACHABLE,
                title = "The host is no longer reachable",
                detail = "The host may have moved out of range or stopped advertising.",
                primaryAction = UserProblemAction.RETURN_TO_SESSIONS,
                secondaryAction = UserProblemAction.RETRY,
                technicalDetail = lastError,
            )

        else -> UserFacingProblem(
            id = "listener-connection-error",
            kind = UserProblemKind.CONNECTION_LOST,
            title = "Could not connect to the host",
            detail = "Move closer to the host, retry, or choose another nearby session.",
            primaryAction = UserProblemAction.RETRY,
            secondaryAction = UserProblemAction.RETURN_TO_SESSIONS,
            technicalDetail = lastError,
        )
    }
}

fun AppUiState.derivedPersistentProblem(): UserFacingProblem? {
    when (storageState) {
        StorageInitializationState.RECOVERABLE_FAILURE -> return UserFacingProblem(
            id = "storage-recoverable",
            kind = UserProblemKind.STORAGE_RECOVERABLE,
            title = "Local app data is temporarily unavailable",
            detail = "Retry opening local app data before hosting or joining.",
            primaryAction = UserProblemAction.RETRY,
            secondaryAction = UserProblemAction.SHARE_SUPPORT_REPORT,
            technicalDetail = storageError,
        )

        StorageInitializationState.FATAL_FAILURE -> return UserFacingProblem(
            id = "storage-fatal",
            kind = UserProblemKind.STORAGE_FATAL,
            title = "Local app data could not be opened",
            detail = "Silent Disco cannot safely continue until this problem is fixed.",
            primaryAction = UserProblemAction.SHARE_SUPPORT_REPORT,
            technicalDetail = storageError,
        )

        StorageInitializationState.INITIALIZING,
        StorageInitializationState.READY -> Unit
    }

    if (listenerState == ListenerLifecycleState.DISCONNECTED) {
        val errorText = lastError.normalizedProblemText()
        if ("host ended" in errorText || "session ended" in errorText) {
            return UserFacingProblem(
                id = "host-ended-session",
                kind = UserProblemKind.HOST_ENDED,
                title = "The host ended the session",
                detail = "Return to nearby sessions to look for another host.",
                primaryAction = UserProblemAction.RETURN_TO_SESSIONS,
                technicalDetail = lastError,
            )
        }
        return UserFacingProblem(
            id = "listener-disconnected",
            kind = UserProblemKind.CONNECTION_LOST,
            title = "Connection lost",
            detail = "The host may have ended the session or moved out of range.",
            primaryAction = UserProblemAction.RECONNECT,
            secondaryAction = UserProblemAction.RETURN_TO_SESSIONS,
            technicalDetail = lastError,
        )
    }

    if (listenerState == ListenerLifecycleState.DESYNCED) {
        return UserFacingProblem(
            id = "listener-desynced",
            kind = UserProblemKind.SYNCHRONIZATION,
            title = "Audio is out of sync",
            detail = "Resynchronize audio to catch up with the host.",
            primaryAction = UserProblemAction.RESYNCHRONIZE,
            secondaryAction = UserProblemAction.RECONNECT,
            technicalDetail = lastError,
        )
    }

    if (listenerPlaybackState == PlaybackState.ERROR) {
        return UserFacingProblem(
            id = "listener-playback",
            kind = UserProblemKind.PLAYBACK,
            title = "Playback problem",
            detail = "Silent Disco could not start or continue audio playback.",
            primaryAction = UserProblemAction.RETRY,
            secondaryAction = UserProblemAction.SHARE_SUPPORT_REPORT,
            technicalDetail = lastError,
        )
    }

    if (listenerState == ListenerLifecycleState.ERROR) {
        return listenerErrorProblem()
    }

    if (hostState == HostLifecycleState.ERROR || hostPlaybackState == PlaybackState.ERROR) {
        return UserFacingProblem(
            id = "host-session",
            kind = UserProblemKind.HOST_SESSION,
            title = "The host session needs attention",
            detail = "Review the problem before continuing the session.",
            primaryAction = UserProblemAction.RETRY,
            secondaryAction = UserProblemAction.SHARE_SUPPORT_REPORT,
            technicalDetail = lastError,
        )
    }

    return null
}
