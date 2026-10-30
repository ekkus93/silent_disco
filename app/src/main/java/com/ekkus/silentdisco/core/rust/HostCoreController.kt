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
import com.ekkus.silentdisco.core.uniffi.FfiQueueEntry
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistSource
import com.ekkus.silentdisco.core.uniffi.FfiTransportState
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout

/** Android-facing command and event port for the authoritative Rust host actor. */
interface HostCoreController : AutoCloseable {
    val snapshots: StateFlow<FfiCoreSnapshot?>
    val notifications: Flow<FfiCoreNotification>

    /** Isolated test doubles may use direct packetization; production reserves
     *  boundaries on its authoritative actor before successor frames enqueue. */
    val boundariesReservedByProducer: Boolean get() = false

    fun openHostPlaylist(
        sessionId: String,
        streamId: String,
        hostStartTimeMs: ULong,
        sources: List<FfiPlaylistSource>,
    ): HostPlaylistStreamController =
        HostPlaylistStreamController.open(sessionId, streamId, hostStartTimeMs, sources)

    suspend fun configureAndCreate(draft: FfiHostDraft)
    suspend fun configureAndCreate(draft: FfiHostDraft, additionalEntries: List<FfiQueueEntry>) {
        configureAndCreate(draft)
    }
    suspend fun appendHostQueue(entries: List<FfiQueueEntry>) {
        throw UnsupportedOperationException("host queue append is unavailable")
    }
    suspend fun reorderHostQueue(orderedEntryIds: List<String>) {
        throw UnsupportedOperationException("host queue reorder is unavailable")
    }
    suspend fun removeHostQueueEntry(entryId: String) {
        throw UnsupportedOperationException("host queue removal is unavailable")
    }
    suspend fun approveJoin(requestId: String, rememberForFuture: Boolean)
    suspend fun rejectJoin(requestId: String)
    suspend fun removeListener(listenerId: String)
    suspend fun endHostSession()
    suspend fun retryRecoverableFailure()
    suspend fun transitionPlaybackState(state: FfiPlaybackState): FfiCoreSnapshot

    fun submitJoinRequest(request: FfiJoinRequestInput)
    fun submitListenerConnected(listener: FfiListenerSummary)
    fun submitListenerDisconnected(deviceId: String)
    fun transportStateChanged(state: FfiTransportState)
    fun transportFailed(message: String, retryable: Boolean)
    fun transportDeliveryCompleted(operationId: String, report: FfiDeliveryReport)
    fun playbackStateChanged(state: FfiPlaybackState)
    fun playbackFailed(message: String)
    fun playbackPositionAdvanced(streamId: String, positionMs: Long)
    fun playbackStreamEnded(streamId: String)
    fun trackBoundaryScheduled(entryId: String, firstSampleIndex: Long)
    fun trackBoundaryApplied(entryId: String, firstSampleIndex: Long)
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
    private val nativeGate = CoreNativeHandleGate(
        open = {
            openInitializedNativeHandle(
                open = { FfiCoreHandle.open(localDeviceId, observer) },
                initialize = { _snapshots.value = it.currentSnapshot() },
                retire = { retireNativeHandle(it, shutdown = { handle -> handle.shutdown() }, close = { handle -> handle.close() }) },
            )
        },
        retire = { retireNativeHandle(it, shutdown = { handle -> handle.shutdown() }, close = { handle -> handle.close() }) },
    )
    private val commandMutex = Mutex()
    private val _snapshots = MutableStateFlow<FfiCoreSnapshot?>(null)
    private val notificationQueue = BoundedCoreNotificationQueue<FfiCoreNotification>()

    private val observer = object : FfiCoreObserver {
        override fun onNotification(notification: FfiCoreNotification) {
            if (closed.get()) return
            when (notification) {
                is FfiCoreNotification.Snapshot -> _snapshots.value = notification.snapshot
                is FfiCoreNotification.Diagnostic -> notificationQueue.publishBestEffort(notification)
                // Platform/transport/storage effects and errors are authoritative.
                // A saturated bridge must backpressure the Rust actor for a bounded
                // interval or fail explicitly, never silently lose an operation.
                else -> notificationQueue.publishCritical(notification)
            }
        }
    }

    override val snapshots: StateFlow<FfiCoreSnapshot?> = _snapshots
    override val notifications: Flow<FfiCoreNotification> = notificationQueue.events
    override val boundariesReservedByProducer: Boolean = true

    override fun openHostPlaylist(
        sessionId: String,
        streamId: String,
        hostStartTimeMs: ULong,
        sources: List<FfiPlaylistSource>,
    ): HostPlaylistStreamController = nativeGate.withOpen { handle ->
        HostPlaylistStreamController.openAuthoritative(
            sessionId, streamId, hostStartTimeMs, sources, handle,
        )
    }

    override suspend fun configureAndCreate(draft: FfiHostDraft) = withContext(Dispatchers.IO) {
        configureAndCreate(draft, emptyList())
    }

    override suspend fun configureAndCreate(draft: FfiHostDraft, additionalEntries: List<FfiQueueEntry>) = withContext(Dispatchers.IO) {
        commandMutex.withLock {
            var snapshot = nativeGate.withOpen { currentSnapshot(it) }
            if (snapshot.selectedRole != FfiAppRole.HOST) {
                val accepted = nativeGate.withOpen {
                    it.selectRole(snapshot.revision, FfiAppRole.HOST).acceptedAtRevision
                }
                snapshot = awaitCommandSnapshot(accepted)
            }
            if (snapshot.hostDraft != draft) {
                val accepted = nativeGate.withOpen {
                    it.updateHostDraft(snapshot.revision, draft).acceptedAtRevision
                }
                snapshot = awaitCommandSnapshot(accepted)
            }
            if (additionalEntries.isNotEmpty()) {
                val accepted = nativeGate.withOpen {
                    it.appendHostQueue(
                        snapshot.revision, snapshot.hostQueueRevision, additionalEntries,
                    ).acceptedAtRevision
                }
                snapshot = awaitCommandSnapshot(accepted)
            }
            nativeGate.withOpen { it.createHostSession(snapshot.revision) }
            Unit
        }
    }

    override suspend fun appendHostQueue(entries: List<FfiQueueEntry>) = submitQueueCommand { handle, snapshot ->
        handle.appendHostQueue(snapshot.revision, snapshot.hostQueueRevision, entries).acceptedAtRevision
    }

    override suspend fun reorderHostQueue(orderedEntryIds: List<String>) = submitQueueCommand { handle, snapshot ->
        handle.reorderHostQueue(snapshot.revision, snapshot.hostQueueRevision, orderedEntryIds).acceptedAtRevision
    }

    override suspend fun removeHostQueueEntry(entryId: String) = submitQueueCommand { handle, snapshot ->
        handle.removeHostQueueEntry(snapshot.revision, snapshot.hostQueueRevision, entryId).acceptedAtRevision
    }

    private suspend fun submitQueueCommand(
        command: (FfiCoreHandle, FfiCoreSnapshot) -> ULong,
    ) = withContext(Dispatchers.IO) {
        commandMutex.withLock {
            val accepted = nativeGate.withOpen { handle ->
                command(handle, currentSnapshot(handle))
            }
            awaitCommandSnapshot(accepted)
            Unit
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

    override suspend fun transitionPlaybackState(state: FfiPlaybackState): FfiCoreSnapshot =
        withContext(Dispatchers.IO) {
            commandMutex.withLock {
                val currentRevision = nativeGate.withOpen { handle ->
                    val revision = currentSnapshot(handle).revision
                    handle.playbackStateChanged(state)
                    revision
                }
                awaitPlaybackSnapshot(currentRevision, state)
            }
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

    override fun playbackFailed(message: String) = withOpenHandle { handle ->
        handle.playbackFailed(message)
    }

    override fun playbackPositionAdvanced(streamId: String, positionMs: Long) =
        withOpenHandle { handle -> handle.playbackPositionAdvanced(streamId, positionMs.toULong()) }

    override fun playbackStreamEnded(streamId: String) =
        withOpenHandle { handle -> handle.playbackStreamEnded(streamId) }

    override fun trackBoundaryScheduled(entryId: String, firstSampleIndex: Long) =
        withOpenHandle { handle -> handle.trackBoundaryScheduled(entryId, firstSampleIndex.toULong()) }

    override fun trackBoundaryApplied(entryId: String, firstSampleIndex: Long) =
        withOpenHandle { handle -> handle.trackBoundaryApplied(entryId, firstSampleIndex.toULong()) }

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
        try {
            nativeGate.close()
        } finally {
            // A failed native shutdown must not strand an observer waiting on
            // the bounded bridge or leave its collector open after close.
            notificationQueue.close()
        }
    }

    private suspend fun submitRevisionCommand(
        command: (FfiCoreHandle, ULong) -> Unit,
    ) = withContext(Dispatchers.IO) {
        commandMutex.withLock {
            nativeGate.withOpen { handle ->
                command(handle, currentSnapshot(handle).revision)
            }
        }
    }

    private fun withOpenHandle(action: (FfiCoreHandle) -> Unit) {
        // All callbacks and native commands share the same lifetime gate as
        // close. A concurrent shutdown must not free the UniFFI handle while
        // a network/actor callback is still invoking it.
        nativeGate.withExisting(action)
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

    private suspend fun awaitPlaybackSnapshot(
        previousRevision: ULong,
        state: FfiPlaybackState,
    ): FfiCoreSnapshot = withTimeout(SNAPSHOT_TIMEOUT_MS) {
        snapshots.filterNotNull().first { snapshot ->
            snapshot.revision > previousRevision && snapshot.playbackState == state
        }
    }

    private companion object {
        const val SNAPSHOT_TIMEOUT_MS = 5_000L
    }
}
