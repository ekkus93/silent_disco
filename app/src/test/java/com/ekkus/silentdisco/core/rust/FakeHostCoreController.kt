package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiCoreNotification
import com.ekkus.silentdisco.core.uniffi.FfiCoreSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiDeliveryReport
import com.ekkus.silentdisco.core.uniffi.FfiHostDraft
import com.ekkus.silentdisco.core.uniffi.FfiJoinRequestInput
import com.ekkus.silentdisco.core.uniffi.FfiListenerSummary
import com.ekkus.silentdisco.core.uniffi.FfiPlatformCompletion
import com.ekkus.silentdisco.core.uniffi.FfiPlaybackState
import com.ekkus.silentdisco.core.uniffi.FfiTransportState
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.receiveAsFlow

/** One recorded `platformOperationFailed`/`storageOperationFailed` call. */
data class RecordedFailure(val operationId: String, val message: String, val retryable: Boolean)

/** One recorded `platformOperationSucceeded` call. */
data class RecordedCompletion(val operationId: String, val completion: FfiPlatformCompletion)

/**
 * A [HostCoreController] test double that records every call made to it, so
 * a test can assert on exactly what an effect-runner function reported back
 * -- not merely that it ran without throwing.
 *
 * [notifications] is backed by a real [Channel] that production code (an
 * `executeRustHostNotification` collector) consumes as it normally would;
 * [emit] lets a test push a notification into that same pipe to drive the
 * production dispatch path end-to-end.
 */
class FakeHostCoreController : HostCoreController {
    private val _snapshots = MutableStateFlow<FfiCoreSnapshot?>(null)
    override val snapshots: StateFlow<FfiCoreSnapshot?> = _snapshots

    private val notificationChannel = Channel<FfiCoreNotification>(Channel.UNLIMITED)
    override val notifications: Flow<FfiCoreNotification> = notificationChannel.receiveAsFlow()

    val configureAndCreateCalls = mutableListOf<FfiHostDraft>()
    val approveJoinCalls = mutableListOf<Pair<String, Boolean>>()
    val rejectJoinCalls = mutableListOf<String>()
    val removeListenerCalls = mutableListOf<String>()
    var endHostSessionCallCount = 0
    var retryRecoverableFailureCallCount = 0
    val transitionPlaybackStateCalls = mutableListOf<FfiPlaybackState>()

    val submitJoinRequestCalls = mutableListOf<FfiJoinRequestInput>()
    val submitListenerConnectedCalls = mutableListOf<FfiListenerSummary>()
    val submitListenerDisconnectedCalls = mutableListOf<String>()
    val transportStateChangedCalls = mutableListOf<FfiTransportState>()
    val transportFailedCalls = mutableListOf<Pair<String, Boolean>>()
    val transportDeliveryCompletedCalls = mutableListOf<Pair<String, FfiDeliveryReport>>()
    val playbackStateChangedCalls = mutableListOf<FfiPlaybackState>()
    val platformOperationSucceededCalls = mutableListOf<RecordedCompletion>()
    val platformOperationFailedCalls = mutableListOf<RecordedFailure>()
    val settingsSavedCalls = mutableListOf<String>()
    val trustedDeviceUpdatedCalls = mutableListOf<Pair<String, String>>()
    val storageOperationFailedCalls = mutableListOf<RecordedFailure>()
    var closeCallCount = 0

    /** Pushes a notification into [notifications], as the real Rust actor would. */
    fun emit(notification: FfiCoreNotification) {
        check(notificationChannel.trySend(notification).isSuccess) {
            "Failed to emit fake host notification $notification"
        }
    }

    override suspend fun configureAndCreate(draft: FfiHostDraft) {
        configureAndCreateCalls += draft
    }

    override suspend fun approveJoin(requestId: String, rememberForFuture: Boolean) {
        approveJoinCalls += requestId to rememberForFuture
    }

    override suspend fun rejectJoin(requestId: String) {
        rejectJoinCalls += requestId
    }

    override suspend fun removeListener(listenerId: String) {
        removeListenerCalls += listenerId
    }

    override suspend fun endHostSession() {
        endHostSessionCallCount += 1
    }

    override suspend fun retryRecoverableFailure() {
        retryRecoverableFailureCallCount += 1
    }

    override suspend fun transitionPlaybackState(state: FfiPlaybackState): FfiCoreSnapshot {
        transitionPlaybackStateCalls += state
        return _snapshots.value ?: error("FakeHostCoreController has no snapshot configured")
    }

    override fun submitJoinRequest(request: FfiJoinRequestInput) {
        submitJoinRequestCalls += request
    }

    override fun submitListenerConnected(listener: FfiListenerSummary) {
        submitListenerConnectedCalls += listener
    }

    override fun submitListenerDisconnected(deviceId: String) {
        submitListenerDisconnectedCalls += deviceId
    }

    override fun transportStateChanged(state: FfiTransportState) {
        transportStateChangedCalls += state
    }

    override fun transportFailed(message: String, retryable: Boolean) {
        transportFailedCalls += message to retryable
    }

    override fun transportDeliveryCompleted(operationId: String, report: FfiDeliveryReport) {
        transportDeliveryCompletedCalls += operationId to report
    }

    override fun playbackStateChanged(state: FfiPlaybackState) {
        playbackStateChangedCalls += state
    }

    override fun platformOperationSucceeded(operationId: String, completion: FfiPlatformCompletion) {
        platformOperationSucceededCalls += RecordedCompletion(operationId, completion)
    }

    override fun platformOperationFailed(operationId: String, message: String, retryable: Boolean) {
        platformOperationFailedCalls += RecordedFailure(operationId, message, retryable)
    }

    override fun settingsSaved(operationId: String) {
        settingsSavedCalls += operationId
    }

    override fun trustedDeviceUpdated(operationId: String, deviceId: String) {
        trustedDeviceUpdatedCalls += operationId to deviceId
    }

    override fun storageOperationFailed(operationId: String, message: String, retryable: Boolean) {
        storageOperationFailedCalls += RecordedFailure(operationId, message, retryable)
    }

    override fun close() {
        closeCallCount += 1
        notificationChannel.close()
    }
}
