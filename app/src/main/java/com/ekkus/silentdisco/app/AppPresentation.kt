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

fun AppUiState.joinUiStep(): JoinUiStep = when (listenerState) {
    ListenerLifecycleState.IDLE,
    ListenerLifecycleState.SCANNING,
    ListenerLifecycleState.SESSION_SELECTED,
    -> JoinUiStep.FINDING_HOST

    ListenerLifecycleState.JOIN_REQUESTED -> JoinUiStep.REQUESTING_ACCESS
    ListenerLifecycleState.AWAITING_APPROVAL -> JoinUiStep.WAITING_FOR_APPROVAL

    ListenerLifecycleState.APPROVED,
    ListenerLifecycleState.CONNECTING,
    ListenerLifecycleState.RECONNECTING,
    ListenerLifecycleState.DISCONNECTED,
    ListenerLifecycleState.ERROR,
    -> JoinUiStep.CONNECTING

    ListenerLifecycleState.SYNCING_CLOCK,
    ListenerLifecycleState.BUFFERING,
    ListenerLifecycleState.DESYNCED,
    -> JoinUiStep.SYNCING_AUDIO

    ListenerLifecycleState.PLAYING -> JoinUiStep.COMPLETE
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
            title = "Playback problem",
            detail = "Silent Disco could not continue playback.",
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
        StorageInitializationState.READY,
        -> Unit
    }

    if (listenerState == ListenerLifecycleState.DISCONNECTED) {
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
