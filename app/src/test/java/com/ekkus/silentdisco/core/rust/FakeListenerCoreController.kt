package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiCoreNotification
import com.ekkus.silentdisco.core.uniffi.FfiCoreSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiPlatformCompletion
import com.ekkus.silentdisco.core.uniffi.FfiSessionAdvertisement
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.receiveAsFlow

/**
 * A [ListenerCoreController] test double that records every call made to
 * it, mirroring [FakeHostCoreController]'s recording shape for the listener
 * role -- see that class's doc comment for the rationale.
 */
class FakeListenerCoreController : ListenerCoreController {
    private val _snapshots = MutableStateFlow<FfiCoreSnapshot?>(null)
    override val snapshots: StateFlow<FfiCoreSnapshot?> = _snapshots

    private val notificationChannel = Channel<FfiCoreNotification>(Channel.UNLIMITED)
    override val notifications: Flow<FfiCoreNotification> = notificationChannel.receiveAsFlow()

    var startDiscoveryCallCount = 0
    var stopDiscoveryCallCount = 0
    val selectSessionCalls = mutableListOf<String>()
    val submitJoinCalls = mutableListOf<String?>()
    var cancelJoinCallCount = 0
    var retryRecoverableFailureCallCount = 0

    val submitSessionDiscoveredCalls = mutableListOf<FfiSessionAdvertisement>()
    val submitSessionExpiredCalls = mutableListOf<String>()
    var submitAwaitingApprovalCallCount = 0
    val submitJoinApprovedCalls = mutableListOf<Boolean>()
    val submitJoinRejectedCalls = mutableListOf<String>()
    val transportFailedCalls = mutableListOf<Pair<String, Boolean>>()
    val platformOperationSucceededCalls = mutableListOf<RecordedCompletion>()
    val platformOperationFailedCalls = mutableListOf<RecordedFailure>()
    var closeCallCount = 0

    /** Pushes a notification into [notifications], as the real Rust actor would. */
    fun emit(notification: FfiCoreNotification) {
        check(notificationChannel.trySend(notification).isSuccess) {
            "Failed to emit fake listener notification $notification"
        }
    }

    override fun startDiscovery() {
        startDiscoveryCallCount += 1
    }

    override fun stopDiscovery() {
        stopDiscoveryCallCount += 1
    }

    override fun selectSession(sessionId: String) {
        selectSessionCalls += sessionId
    }

    override fun submitJoin(inviteCode: String?) {
        submitJoinCalls += inviteCode
    }

    override fun cancelJoin() {
        cancelJoinCallCount += 1
    }

    override fun retryRecoverableFailure() {
        retryRecoverableFailureCallCount += 1
    }

    override fun submitSessionDiscovered(session: FfiSessionAdvertisement) {
        submitSessionDiscoveredCalls += session
    }

    override fun submitSessionExpired(sessionId: String) {
        submitSessionExpiredCalls += sessionId
    }

    override fun submitAwaitingApproval() {
        submitAwaitingApprovalCallCount += 1
    }

    override fun submitJoinApproved(trustedForFuture: Boolean) {
        submitJoinApprovedCalls += trustedForFuture
    }

    override fun submitJoinRejected(reason: String) {
        submitJoinRejectedCalls += reason
    }

    override fun transportFailed(message: String, retryable: Boolean) {
        transportFailedCalls += message to retryable
    }

    override fun platformOperationSucceeded(operationId: String, completion: FfiPlatformCompletion) {
        platformOperationSucceededCalls += RecordedCompletion(operationId, completion)
    }

    override fun platformOperationFailed(operationId: String, message: String, retryable: Boolean) {
        platformOperationFailedCalls += RecordedFailure(operationId, message, retryable)
    }

    override fun close() {
        closeCallCount += 1
        notificationChannel.close()
    }
}
