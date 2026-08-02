package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiAppRole
import com.ekkus.silentdisco.core.uniffi.FfiCoreHandle
import com.ekkus.silentdisco.core.uniffi.FfiCoreNotification
import com.ekkus.silentdisco.core.uniffi.FfiCoreObserver
import com.ekkus.silentdisco.core.uniffi.FfiCoreSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiListenerLifecycle
import com.ekkus.silentdisco.core.uniffi.FfiPlatformCompletion
import com.ekkus.silentdisco.core.uniffi.FfiSessionAdvertisement
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.receiveAsFlow

/** Android-facing command and event port for the authoritative Rust listener actor. */
interface ListenerCoreController : AutoCloseable {
    val snapshots: StateFlow<FfiCoreSnapshot?>
    val notifications: Flow<FfiCoreNotification>

    fun startDiscovery()
    fun stopDiscovery()
    fun selectSession(sessionId: String)
    fun submitJoin(inviteCode: String?)
    fun cancelJoin()
    fun retryRecoverableFailure()

    fun submitSessionDiscovered(session: FfiSessionAdvertisement)
    fun submitSessionExpired(sessionId: String)
    fun submitAwaitingApproval()
    fun submitJoinApproved(trustedForFuture: Boolean)
    fun submitJoinRejected(reason: String)
    fun transportFailed(message: String, retryable: Boolean)
    fun platformOperationSucceeded(operationId: String, completion: FfiPlatformCompletion)
    fun platformOperationFailed(operationId: String, message: String, retryable: Boolean)
}

/**
 * UniFFI implementation that opens its own [FfiCoreHandle], independent of any host
 * controller. This PoC drives one role at a time per physical device, so a second,
 * independently-opened actor for the listener role (rather than sharing the host's
 * handle) keeps this addition from touching the already device-verified host path.
 */
class UniFfiListenerCoreController(
    private val localDeviceId: String,
) : ListenerCoreController {
    private val closed = AtomicBoolean(false)
    private val handleRef = AtomicReference<FfiCoreHandle?>(null)
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
                            "SilentDisco: Rust listener notification could not be queued: " +
                                notification::class.java.simpleName,
                        )
                    }
                }
            }
        }
    }

    override val snapshots: StateFlow<FfiCoreSnapshot?> = _snapshots
    override val notifications: Flow<FfiCoreNotification> = notificationChannel.receiveAsFlow()

    override fun startDiscovery() = withRoleSelected { handle, revision ->
        handle.startDiscovery(revision)
    }

    override fun stopDiscovery() = withOpenHandle { handle ->
        handle.stopDiscovery(currentSnapshot(handle).revision)
    }

    override fun selectSession(sessionId: String) = withOpenHandle { handle ->
        handle.selectSession(currentSnapshot(handle).revision, sessionId)
    }

    override fun submitJoin(inviteCode: String?) = withOpenHandle { handle ->
        handle.submitJoin(currentSnapshot(handle).revision, inviteCode)
    }

    override fun cancelJoin() = withOpenHandle { handle ->
        handle.cancelJoin(currentSnapshot(handle).revision)
    }

    override fun retryRecoverableFailure() = withOpenHandle { handle ->
        handle.retryRecoverableFailure(currentSnapshot(handle).revision)
    }

    override fun submitSessionDiscovered(session: FfiSessionAdvertisement) = withOpenHandle { handle ->
        handle.submitSessionDiscovered(session)
    }

    override fun submitSessionExpired(sessionId: String) = withOpenHandle { handle ->
        handle.submitSessionExpired(sessionId)
    }

    override fun submitAwaitingApproval() = withOpenHandle { handle ->
        handle.submitAwaitingApproval()
    }

    override fun submitJoinApproved(trustedForFuture: Boolean) = withOpenHandle { handle ->
        handle.submitJoinApproved(trustedForFuture)
    }

    override fun submitJoinRejected(reason: String) = withOpenHandle { handle ->
        handle.submitJoinRejected(reason)
    }

    override fun transportFailed(message: String, retryable: Boolean) = withOpenHandle { handle ->
        handle.transportFailed(message, retryable)
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

    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        val handle = handleRef.getAndSet(null)
        if (handle != null) {
            runCatching { handle.shutdown() }
            handle.close()
        }
        notificationChannel.close()
    }

    private inline fun withRoleSelected(command: (FfiCoreHandle, ULong) -> Unit) {
        val handle = openHandle()
        var snapshot = currentSnapshot(handle)
        if (snapshot.selectedRole != FfiAppRole.LISTENER) {
            handle.selectRole(snapshot.revision, FfiAppRole.LISTENER)
            snapshot = handle.currentSnapshot().also { _snapshots.value = it }
        }
        command(handle, snapshot.revision)
    }

    private fun openHandle(): FfiCoreHandle {
        handleRef.get()?.let { return it }
        check(!closed.get()) { "Rust listener controller is closed" }
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

    private fun currentSnapshot(handle: FfiCoreHandle): FfiCoreSnapshot =
        _snapshots.value ?: handle.currentSnapshot().also { _snapshots.value = it }
}
