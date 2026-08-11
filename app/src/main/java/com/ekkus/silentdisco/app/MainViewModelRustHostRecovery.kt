package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.rust.HostCoreController
import com.ekkus.silentdisco.core.uniffi.FfiCoreNotification
import com.ekkus.silentdisco.core.uniffi.FfiDeliveryReport
import com.ekkus.silentdisco.core.uniffi.FfiPlatformEffect
import com.ekkus.silentdisco.core.uniffi.FfiStorageEffect
import com.ekkus.silentdisco.core.uniffi.FfiTransportEffect

private const val MAX_RUST_HOST_FAILURE_MESSAGE_BYTES = 400
private const val MAX_RUST_HOST_FAILURE_DETAIL_BYTES = 200

/**
 * Recovery boundary for an unexpected exception while executing one Rust host
 * notification. The notification collector itself must stay alive: one bad
 * Android platform callback must not silently disable every later host effect.
 *
 * Correlated operations are explicitly completed as failures so the Rust
 * actor does not retain an in-flight operation forever. Host start/stop also
 * receives effect-specific platform cleanup before the failure is reported.
 */
internal fun MainViewModel.recoverRustHostNotificationFailure(
    controller: HostCoreController,
    notification: FfiCoreNotification,
    error: Exception,
) {
    val cleanupFailures = when (notification) {
        is FfiCoreNotification.PlatformEffect -> when (notification.effect) {
            is FfiPlatformEffect.StartAdvertising -> rollbackFailedRustHostStart()
            is FfiPlatformEffect.StopAdvertising -> finishFailedRustHostStopCleanup()
            else -> emptyList()
        }
        else -> emptyList()
    }
    val category = when (notification) {
        is FfiCoreNotification.PlatformEffect -> "platform effect"
        is FfiCoreNotification.TransportEffect -> "transport effect"
        is FfiCoreNotification.StorageEffect -> "storage effect"
        is FfiCoreNotification.Snapshot -> "snapshot notification"
        is FfiCoreNotification.Error -> "error notification"
        is FfiCoreNotification.Diagnostic -> "diagnostic notification"
    }
    val cleanupSuffix = cleanupFailures.takeIf { it.isNotEmpty() }
        ?.joinToString(prefix = "; cleanup also failed: ", separator = "; ")
        .orEmpty()
    val message = (
        "Rust host $category failed unexpectedly: ${error.safeRustHostFailureDetail()}$cleanupSuffix"
    ).sanitizedAndBounded(MAX_RUST_HOST_FAILURE_MESSAGE_BYTES)

    surfaceRustHostNotificationFailure(message, error)

    val reportFailure = runCatching {
        when (notification) {
            is FfiCoreNotification.PlatformEffect -> controller.platformOperationFailed(
                notification.effect.operationId(),
                message,
                false,
            )
            is FfiCoreNotification.TransportEffect -> controller.transportDeliveryCompleted(
                notification.effect.operationId(),
                FfiDeliveryReport(
                    intendedPeers = 1u,
                    successfulPeers = 0u,
                    failedPeers = 1u,
                ),
            )
            is FfiCoreNotification.StorageEffect -> controller.storageOperationFailed(
                notification.effect.operationId(),
                message,
                false,
            )
            is FfiCoreNotification.Snapshot,
            is FfiCoreNotification.Error,
            is FfiCoreNotification.Diagnostic,
            -> Unit
        }
    }.exceptionOrNull()

    if (reportFailure != null) {
        val reportMessage = (
            "$message; Rust recovery report failed: ${reportFailure.safeRustHostFailureDetail()}"
        ).sanitizedAndBounded(MAX_RUST_HOST_FAILURE_MESSAGE_BYTES)
        surfaceRustHostNotificationFailure(reportMessage, reportFailure)
    }
}

private fun MainViewModel.rollbackFailedRustHostStart(): List<String> {
    pendingStartAdvertisingOperationId = null
    pendingListenerDatagramPorts.clear()
    currentSessionId = null
    currentStreamId = null
    latestPackets = emptyList()
    _uiState.value = _uiState.value.copy(discoveredSessions = emptyList())
    return collectRustHostCleanupFailures(
        "stop BLE advertising" to { bleService.stopAdvertising() },
        "stop Wi-Fi Direct host" to { wifiDirectService.stop() },
    )
}

private fun MainViewModel.finishFailedRustHostStopCleanup(): List<String> {
    hostStreamJob?.cancel()
    playbackJob?.cancel()
    resyncJob?.cancel()
    pendingStartAdvertisingOperationId = null
    pendingListenerDatagramPorts.clear()
    currentSessionId = null
    currentStreamId = null
    latestPackets = emptyList()
    _uiState.value = _uiState.value.copy(discoveredSessions = emptyList())
    return collectRustHostCleanupFailures(
        "stop playback" to { playbackEngine.stop() },
        "stop BLE" to { bleService.stop() },
        "stop Wi-Fi Direct" to { wifiDirectService.stop() },
        "close host transport" to { hostTransportController.close() },
    )
}

private fun MainViewModel.collectRustHostCleanupFailures(
    vararg cleanupActions: Pair<String, () -> Unit>,
): List<String> = buildList {
    cleanupActions.forEach { (label, action) ->
        runCatching(action).exceptionOrNull()?.let { cleanupError ->
            val detail = cleanupError.safeRustHostFailureDetail()
            logger.e("rust.host.notification.cleanup", "$label failed: $detail", cleanupError)
            add("$label: $detail")
        }
    }
}

private fun MainViewModel.surfaceRustHostNotificationFailure(message: String, error: Throwable) {
    logger.e("rust.host.notification", message, error)
    _uiState.value = _uiState.value.copy(lastError = message)
    val diagnosticsFailure = runCatching {
        diagnosticsStore.updateHost {
            it.copy(
                lastError = message,
                metricsSummary = summarizeMetrics(),
            )
        }
        refreshHostDiagnostics(
            streamState = _uiState.value.hostPlaybackState,
            sessionId = currentSessionId?.value ?: _uiState.value.hostDiagnostics.sessionId,
        )
    }.exceptionOrNull()
    if (diagnosticsFailure != null) {
        logger.e(
            "rust.host.notification.diagnostics",
            "Failed to update host diagnostics after notification failure: " +
                diagnosticsFailure.safeRustHostFailureDetail(),
            diagnosticsFailure,
        )
    }
}

private fun FfiPlatformEffect.operationId(): String = when (this) {
    is FfiPlatformEffect.StartAdvertising -> operationId
    is FfiPlatformEffect.StopAdvertising -> operationId
    is FfiPlatformEffect.StartDiscovery -> operationId
    is FfiPlatformEffect.StopDiscovery -> operationId
    is FfiPlatformEffect.EstablishNetwork -> operationId
    is FfiPlatformEffect.ReleaseNetwork -> operationId
    is FfiPlatformEffect.RequestCapabilities -> operationId
    is FfiPlatformEffect.PrepareAudioSource -> operationId
    is FfiPlatformEffect.StartAudioOutput -> operationId
    is FfiPlatformEffect.StopAudioOutput -> operationId
    is FfiPlatformEffect.ShareDiagnostics -> operationId
}

private fun FfiTransportEffect.operationId(): String = when (this) {
    is FfiTransportEffect.DeliverJoinApproval -> operationId
    is FfiTransportEffect.DeliverJoinRejection -> operationId
    is FfiTransportEffect.DisconnectListener -> operationId
}

private fun FfiStorageEffect.operationId(): String = when (this) {
    is FfiStorageEffect.PersistSettings -> operationId
    is FfiStorageEffect.PersistTrustedDevice -> operationId
}

private fun Throwable.safeRustHostFailureDetail(): String {
    val raw = message?.takeIf { it.isNotBlank() } ?: javaClass.simpleName
    return raw.sanitizedAndBounded(MAX_RUST_HOST_FAILURE_DETAIL_BYTES)
        .ifBlank { javaClass.simpleName }
}

private fun String.sanitizedAndBounded(maxBytes: Int): String {
    val sanitized = buildString(length) {
        this@sanitizedAndBounded.forEach { character ->
            append(if (character.isISOControl()) ' ' else character)
        }
    }.trim()
    if (sanitized.toByteArray(Charsets.UTF_8).size <= maxBytes) return sanitized

    val result = StringBuilder()
    var bytes = 0
    sanitized.forEach { character ->
        val characterBytes = character.toString().toByteArray(Charsets.UTF_8).size
        if (bytes + characterBytes > maxBytes) return result.toString().trimEnd()
        result.append(character)
        bytes += characterBytes
    }
    return result.toString().trimEnd()
}
