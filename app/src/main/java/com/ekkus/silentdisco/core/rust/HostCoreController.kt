package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiAppRole
import com.ekkus.silentdisco.core.uniffi.FfiCoreHandle
import com.ekkus.silentdisco.core.uniffi.FfiCoreNotification
import com.ekkus.silentdisco.core.uniffi.FfiCoreObserver
import com.ekkus.silentdisco.core.uniffi.FfiCoreSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiDeliveryReport
import com.ekkus.silentdisco.core.uniffi.FfiHostDraft
import com.ekkus.silentdisco.core.uniffi.FfiHostLifecycle
import com.ekkus.silentdisco.core.uniffi.FfiJoinRequestInput
import com.ekkus.silentdisco.core.uniffi.FfiListenerSummary
import com.ekkus.silentdisco.core.uniffi.FfiPlatformCompletion
import com.ekkus.silentdisco.core.uniffi.FfiPlaybackState
import com.ekkus.silentdisco.core.uniffi.FfiTransportState
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
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
class UniFfiHostCoreController(
    private val localDeviceId: String,
) : HostCoreController {
    private val closed = AtomicBoolean(false)
    private val handleRef = AtomicReference<FfiCoreHandle?>(null)
    private val commandMutex = Mutex()
    private val _snapshots = MutableStateFlow<FfiCoreSnapshot?>(null)
    private val notificationChannel = Channel<FfiCoreNotification>(Channel.UNLIMITED)

    private val observer = object : FfiCoreObserver {
        override fun onNotification(notification: FfiCoreNotification) {
            when (notification) {
                is FfiCoreNotification.Snapshot -> _snapshots.value = notification.snapshot
                else -> {
                    val delivered = notificationChannel.trySend(notification).isSuccess
                    if (!delivered && !closed.get()) {
                        System.err.println(
                            "SilentDisco: Rust host notification could not be queued: " +
                                notification::class.java.simpleName,
                        )
                    }
                }
            }
        }
    }

    override val snapshots: StateFlow<FfiCoreSnapshot?> = _snapshots
    override val notifications: Flow<FfiCoreNotification> = notificationChannel.receiveAsFlow()

    override suspend fun configureAndCreate(draft: FfiHostDraft) = withContext(Dispatchers.IO) {
        commandMutex.withLock {
            val handle = openHandle()
            var snapshot = currentSnapshot(handle)
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
        }
    }

    override suspend fun approveJoin(requestId: String, rememberForFuture: Boolean) =
        submitRevisionCommand { handle, revision ->
            handle.approveJoin(revision, requestId, rememberForFuture)
        }

    override suspend fun rejectJoin(requestId: String) = submitRevisionCommand { handle, revision ->
        handle.rejectJoin(revision, requestId)
    }

    override suspend fun removeListener(listenerId: String) = submitRevisionCommand { handle, revision ->
        handle.removeListener(revision, listenerId)
    }

    override suspend fun endHostSession() = submitRevisionCommand { handle, revision ->
        handle.endHostSession(revision)
    }

    override suspend fun retryRecoverableFailure() = submitRevisionCommand { handle, revision ->
        handle.retryRecoverableFailure(revision)
    }

    override fun submitJoinRequest(request: FfiJoinRequestInput) = withOpenHandle { handle ->
        handle.submitJoinRequest(request)
    }

    override fun submitListenerConnected(listener: FfiListenerSummary) = withOpenHandle { handle ->
        handle.submitListenerConnected(listener)
    }

    override fun submitListenerDisconnected(deviceId: String) = withOpenHandle { handle ->
        handle.submitListenerDisconnected(deviceId, null)
    }

    override fun transportStateChanged(state: FfiTransportState) {
        if (!hasActiveHostLifecycle()) return
        withOpenHandle { handle -> handle.transportStateChanged(state) }
    }

    override fun transportFailed(message: String, retryable: Boolean) {
        if (!hasActiveHostLifecycle()) return
        withOpenHandle { handle -> handle.transportFailed(message, retryable) }
    }

    override fun transportDeliveryCompleted(operationId: String, report: FfiDeliveryReport) =
        withOpenHandle { handle -> handle.transportDeliveryCompleted(operationId, report) }

    override fun playbackStateChanged(state: FfiPlaybackState) = withOpenHandle { handle ->
        handle.playbackStateChanged(state)
    }

    override fun platformOperationSucceeded(
        operationId: String,
        completion: FfiPlatformCompletion,
    ) = withOpenHandle { handle -> handle.platformOperationSucceeded(operationId, completion) }

    override fun platformOperationFailed(
        operationId: String,
        message: String,
        retryable: Boolean,
    ) = withOpenHandle { handle ->
        handle.platformOperationFailed(operationId, message, retryable)
    }

    override fun settingsSaved(operationId: String) = withOpenHandle { handle ->
        handle.settingsSaved(operationId)
    }

    override fun trustedDeviceUpdated(operationId: String, deviceId: String) =
        withOpenHandle { handle -> handle.trustedDeviceUpdated(operationId, deviceId) }

    override fun storageOperationFailed(
        operationId: String,
        message: String,
        retryable: Boolean,
    ) = withOpenHandle { handle ->
        handle.storageOperationFailed(operationId, message, retryable)
    }

    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        val handle = handleRef.getAndSet(null)
        if (handle != null) {
            runCatching { handle.shutdown() }
            handle.close()
        }
        notificationChannel.close()
    }

    private suspend fun submitRevisionCommand(
        command: (FfiCoreHandle, ULong) -> Unit,
    ) = withContext(Dispatchers.IO) {
        commandMutex.withLock {
            val handle = openHandle()
            command(handle, currentSnapshot(handle).revision)
        }
    }

    private fun openHandle(): FfiCoreHandle {
        handleRef.get()?.let { return it }
        check(!closed.get()) { "Rust host controller is closed" }
        return synchronized(handleRef) {
            handleRef.get() ?: FfiCoreHandle.open(localDeviceId, observer).also { opened ->
                _snapshots.value = opened.currentSnapshot()
                handleRef.set(opened)
            }
        }
    }

    private inline fun withOpenHandle(action: (FfiCoreHandle) -> Unit) {
        handleRef.get()?.let(action)
    }

    private fun hasActiveHostLifecycle(): Boolean {
        val snapshot = _snapshots.value ?: return false
        return snapshot.selectedRole == FfiAppRole.HOST &&
            snapshot.hostLifecycle != FfiHostLifecycle.IDLE
    }

    private fun currentSnapshot(handle: FfiCoreHandle): FfiCoreSnapshot =
        _snapshots.value ?: handle.currentSnapshot().also { _snapshots.value = it }

    private suspend fun awaitCommandSnapshot(acceptedAtRevision: ULong): FfiCoreSnapshot =
        withTimeout(SNAPSHOT_TIMEOUT_MS) {
            snapshots.filterNotNull().first { it.revision > acceptedAtRevision }
        }

    private companion object {
        const val SNAPSHOT_TIMEOUT_MS = 5_000L
    }
}
