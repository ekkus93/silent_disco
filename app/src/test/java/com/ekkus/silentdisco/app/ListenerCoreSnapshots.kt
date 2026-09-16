package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.uniffi.FfiApprovalMode
import com.ekkus.silentdisco.core.uniffi.FfiCoreSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiHostDraft
import com.ekkus.silentdisco.core.uniffi.FfiHostLifecycle
import com.ekkus.silentdisco.core.uniffi.FfiListenerLifecycle
import com.ekkus.silentdisco.core.uniffi.FfiPlaybackState
import com.ekkus.silentdisco.core.uniffi.FfiSessionAdvertisement
import com.ekkus.silentdisco.core.uniffi.FfiTransportState
import com.ekkus.silentdisco.core.uniffi.FfiTuningSettings

/**
 * A minimal listener-role core snapshot for tests: [known] session IDs are
 * listed as discovered, and [selected] (when set) is a joined session in
 * `CONNECTING`.
 */
internal fun listenerCoreSnapshot(
    discoveryActive: Boolean,
    known: List<String>,
    selected: String?,
) = FfiCoreSnapshot(
    revision = 1uL,
    selectedRole = null,
    hostDraft = FfiHostDraft(
        sessionName = "",
        approvalMode = FfiApprovalMode.MANUAL,
        inviteCode = null,
        audioSource = null,
        rememberApprovedDevices = false,
        tuning = FfiTuningSettings(
            syncSampleWindow = 12u,
            syncCadenceMs = 2_000uL,
            startupBufferMs = 400uL,
            latePacketThresholdMs = 60uL,
            hardResyncThresholdMs = 120uL,
            syncDriftThresholdMs = 12.0,
            scanWindowMs = 4_000uL,
        ),
    ),
    hostLifecycle = FfiHostLifecycle.IDLE,
    listenerLifecycle = if (selected != null) {
        FfiListenerLifecycle.CONNECTING
    } else if (discoveryActive) {
        FfiListenerLifecycle.SCANNING
    } else {
        FfiListenerLifecycle.IDLE
    },
    transportState = if (discoveryActive) FfiTransportState.DISCOVERING else FfiTransportState.IDLE,
    discoveryActive = discoveryActive,
    discoveredSessions = known.map {
        FfiSessionAdvertisement(
            sessionId = it,
            hostDeviceId = "desktop-host",
            sessionName = "Session $it",
            approvalMode = FfiApprovalMode.MANUAL,
            protocolVersion = 2u,
            address = "192.168.1.50",
            controlPort = 41_000u,
            syncPort = 41_001u,
            audioPort = 41_002u,
        )
    },
    selectedSession = selected,
    pendingJoinRequests = emptyList(),
    listeners = emptyList(),
    playbackState = FfiPlaybackState.STOPPED,
    playbackPositionMs = 0uL,
    streamEndedNaturally = false,
    lastDelivery = null,
    recoverableAction = null,
    lastError = null,
    shuttingDown = false,
)
