package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiAppRole
import com.ekkus.silentdisco.core.uniffi.FfiCoreHandle
import com.ekkus.silentdisco.core.uniffi.FfiCoreNotification
import com.ekkus.silentdisco.core.uniffi.FfiCoreObserver
import com.ekkus.silentdisco.core.uniffi.FfiCoreSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiDeliveryReport
import com.ekkus.silentdisco.core.uniffi.FfiHostDraft
import com.ekkus.silentdisco.core.uniffi.FfiJoinRequestInput
import com.ekkus.silentdisco.core.uniffi.FfiListenerSummary
import com.ekkus.silentdisco.core.uniffi.FfiPlatformCompletion
import com.ekkus.silentdisco.core.uniffi.FfiPlaybackState
import com.ekkus.silentdisco.core.uniffi.FfiTransportState
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeout

/** Android-facing command and event port for the authoritative Rust host actor. */
interface HostCoreController : AutoCloseable {
    val snapshots: StateFlow<FfiCoreSnapshot?>
    val notifications: Flow<FfiCoreNotification>

    suspend fun configureAndCreate(draft: FfiHostDraft)
    suspend fun approveJoin(requestId: String, rememberForFuture: Boolean)
    suspend fun rejectJoin(requestId: String)
    suspend fun removeListener(listenerId: String)
    suspend fun endHostSession()
    suspend fun retryRecoverableFailure()

    fun submitJoinRequest(request: FfiJoinRequestInput)
    fun submitListenerConnected(listener: FfiListenerSummary)
    fun submitListenerDisconnected(deviceId: String)
    fun transportStateChanged(state: FfiTransportState)
    fun transportFailed(message: String, retryable: Boolean)
    fun transportDeliveryCompleted(operationId: String, report: FfiDeliveryReport)
    fun playbackStateChanged(state: FfiPlaybackState)
    fun platformOperationSucceeded(operationId: String, completion: FfiPlatformCompletion)
    fun platformOperationFailed(operationId: String, message: String, retryable: Boolean)
    fun settingsSaved(operationId: String)
    fun trustedDeviceUpdated(operationId: String, deviceId: String)
    fun storageOperationFailed(operationId: String, message: String, retryable: Boolean)
}

/** UniFFI implementation that serializes revision-bearing commands. */
class UniFfiHostCoreController(localDeviceId: String) : HostCoreController {
    private val closed = AtomicBoolean(false)
    private val commandMutex = Mutex()
    private val _snapshots = MutableStateFlow<FfiCoreSnapshot?>(null)
    private val notificationChannel = Channel<FfiCoreNotification>(Channel.UNLIMITED)

    private val observer = object : FfiCoreObserver {
        override fun onNotification(notification: FfiCoreNotification) {
            when (notification) {
                is FfiCoreNotification.Snapshot -> _snapshots.value = notification.snapshot
                else -> check(notificationChannel.trySend(notification).isSuccess) {
                    "Rust host notification channel is closed"
                }
            }
        }
    }

    private val handle = FfiCoreHandle.open(localDeviceId, observer)

    override val snapshots: StateFlow<FfiCoreSnapshot?> = _snapshots
    override val notifications: Flow<FfiCoreNotification> = notificationChannel.receiveAsFlow()

    init {
        _snapshots.value = handle.currentSnapshot()
    }

    override suspend fun configureAndCreate(draft: FfiHostDraft) = commandMutex.withLock {
        var snapshot = currentSnapshot()
        if (snapshot.selectedRole != FfiAppRole.HOST) {
            snapshot = awaitCommandSnapshot(
                handle.selectRole(snapshot.revision, FfiAppRole.HOST).acceptedAtRevision,
            )
        }
        if (snapshot.hostDraft != draft) {
            snapshot = awaitCommandSnapshot(
                handle.updateHostDraft(snapshot.revision, draft).acceptedAtRevision,
            )
        }
        handle.createHostSession(snapshot.revision)
        Unit
    }

    override suspend fun approveJoin(requestId: String, rememberForFuture: Boolean) =
        submitRevisionCommand { revision ->
            handle.approveJoin(revision, requestId, rememberForFuture)
        }

    override suspend fun rejectJoin(requestId: String) = submitRevisionCommand { revision ->
        handle.rejectJoin(revision, requestId)
    }

    override suspend fun removeListener(listenerId: String) = submitRevisionCommand { revision ->
        handle.removeListener(revision, listenerId)
    }

    override suspend fun endHostSession() = submitRevisionCommand { revision ->
        handle.endHostSession(revision)
    }

    override suspend fun retryRecoverableFailure() = submitRevisionCommand { revision ->
        handle.retryRecoverableFailure(revision)
    }

    override fun submitJoinRequest(request: FfiJoinRequestInput) = handle.submitJoinRequest(request)

    override fun submitListenerConnected(listener: FfiListenerSummary) =
        handle.submitListenerConnected(listener)

    override fun submitListenerDisconnected(deviceId: String) =
        handle.submitListenerDisconnected(deviceId, null)

    override fun transportStateChanged(state: FfiTransportState) = handle.transportStateChanged(state)

    override fun transportFailed(message: String, retryable: Boolean) =
        handle.transportFailed(message, retryable)

    override fun transportDeliveryCompleted(operationId: String, report: FfiDeliveryReport) =
        handle.transportDeliveryCompleted(operationId, report)

    override fun playbackStateChanged(state: FfiPlaybackState) = handle.playbackStateChanged(state)

    override fun platformOperationSucceeded(
        operationId: String,
        completion: FfiPlatformCompletion,
    ) = handle.platformOperationSucceeded(operationId, completion)

    override fun platformOperationFailed(
        operationId: String,
        message: String,
        retryable: Boolean,
    ) = handle.platformOperationFailed(operationId, message, retryable)

    override fun settingsSaved(operationId: String) = handle.settingsSaved(operationId)

    override fun trustedDeviceUpdated(operationId: String, deviceId: String) =
        handle.trustedDeviceUpdated(operationId, deviceId)

    override fun storageOperationFailed(
        operationId: String,
        message: String,
        retryable: Boolean,
    ) = handle.storageOperationFailed(operationId, message, retryable)

    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        runCatching { handle.shutdown() }
        notificationChannel.close()
        handle.close()
    }

    private suspend fun submitRevisionCommand(
        command: (ULong) -> Unit,
    ) = commandMutex.withLock {
        command(currentSnapshot().revision)
    }

    private fun currentSnapshot(): FfiCoreSnapshot =
        _snapshots.value ?: handle.currentSnapshot().also { _snapshots.value = it }

    private suspend fun awaitCommandSnapshot(acceptedAtRevision: ULong): FfiCoreSnapshot =
        withTimeout(SNAPSHOT_TIMEOUT_MS) {
            snapshots.filterNotNull().first { it.revision > acceptedAtRevision }
        }

    private companion object {
        const val SNAPSHOT_TIMEOUT_MS = 5_000L
    }
}
