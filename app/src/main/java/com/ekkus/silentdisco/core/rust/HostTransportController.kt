package com.ekkus.silentdisco.core.rust

import android.os.SystemClock
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportDelivery
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportException
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportHandle
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private const val POLL_TIMEOUT_MS: ULong = 500uL

/** Legacy v3 has no handshake-verified listener key; a claimed ID is never durable trust. */
internal const val LEGACY_V3_DURABLE_TRUST_ERROR =
    "Legacy v3 transport cannot remember a listener without verified cryptographic identity"

/**
 * Android-facing wrapper around the shared Rust host transport for one
 * hosted session at a time. Owns no domain policy: it only binds the real
 * transport, exposes its send/broadcast methods, and surfaces the
 * transport's own typed events as a [Flow] -- the caller (`MainViewModelRustHost`)
 * bridges those events into the existing `HostCoreController`/`FfiCoreHandle`
 * actor, mirroring how `MainViewModelRustListener` bridges
 * `FfiListenerTransportEvent` into `ListenerCoreController`.
 *
 * The controller itself is reusable across sequential hosted sessions. [close]
 * releases the current native session and poll job but intentionally keeps the
 * event stream open; `MainViewModel` installs one collector for its lifetime.
 */
class HostTransportController(
    /**
     * Runs bind and send calls. The event poll loop always stays on
     * [Dispatchers.IO] because it blocks inside native `pollEvent`.
     */
    private val callDispatcher: CoroutineDispatcher = Dispatchers.IO,
    /** Opens the native host transport; replaced only by JVM tests that record broadcasts. */
    private val bindHandle: (String, UShort, UShort, UShort, String) -> FfiHostTransportHandle =
        { localAddress, controlPort, syncPort, audioPort, sessionId ->
            FfiHostTransportHandle.bindWithHostClock(
                localAddress, controlPort, syncPort, audioPort, sessionId,
                SystemClock.elapsedRealtime().toULong(),
            )
        },
) : AutoCloseable {
    private val handleGate = HostNativeHandleGate<FfiHostTransportHandle>()
    private var eventLoop: Job? = null
    // The channel survives session rebinds. Tag queued events with their
    // original handle so a slow collector cannot consume previous-session
    // admission or disconnect events after a new host is bound.
    private val eventQueue = BoundedTransportEventQueue<FfiHostTransportHandle, FfiHostTransportEvent>(
        currentHandle = handleGate::currentHandle,
    )

    val events: Flow<FfiHostTransportEvent> = eventQueue.events
    /** Best-effort diagnostic events dropped while the UI collector was slow. */
    val droppedDiagnosticEvents: Long get() = eventQueue.droppedBestEffort

    suspend fun bind(
        scope: CoroutineScope,
        localAddress: String,
        controlPort: Int,
        syncPort: Int,
        audioPort: Int,
        sessionId: String,
    ) {
        withContext(callDispatcher) {
            handleGate.replace(
                retire = ::retireHandle,
                open = {
                    bindHandle(
                        localAddress,
                        controlPort.toUShort(),
                        syncPort.toUShort(),
                        audioPort.toUShort(),
                        sessionId,
                    )
                },
                start = { handle ->
                    // Old-session events must not occupy the new session's bounded queue.
                    eventQueue.discardRetiredEvents()
                    startEventLoop(scope, handle)
                },
            )
        }
    }

    suspend fun sendJoinApproval(listenerId: String, trustedForFuture: Boolean): FfiHostTransportDelivery? {
        check(!trustedForFuture) { LEGACY_V3_DURABLE_TRUST_ERROR }
        return withHandle { it.sendJoinApproval(listenerId, trustedForFuture = false) }
    }

    /**
     * Complete approval and datagram-route authorization under one handle gate.
     * Capturing the generation before dispatch prevents a delayed approval
     * from being applied to a new host after Stop/rebind.
     */
    internal suspend fun approveAndAuthorizeListener(
        listenerId: String,
        trustedForFuture: Boolean,
        syncPort: UShort,
        audioPort: UShort,
    ): HostJoinAuthorizationResult<FfiHostTransportDelivery>? {
        // A manually approved plaintext-v3 listener may join this session, but
        // must never be promoted to durable trust using a self-claimed ID.
        check(!trustedForFuture) { LEGACY_V3_DURABLE_TRUST_ERROR }
        val expected = handleGate.currentHandle() ?: return null
        return withContext(callDispatcher) {
            handleGate.deliverAndAuthorize(
                expected = expected,
                deliver = { it.sendJoinApproval(listenerId, trustedForFuture) },
                wasDelivered = { it.successfulPeers > 0u },
                authorize = { it.authorizeListener(listenerId, syncPort, audioPort) },
                disconnect = { it.disconnectPeer(listenerId, "Listener route authorization failed") },
            )
        }
    }

    suspend fun sendJoinRejection(listenerId: String, reason: String): FfiHostTransportDelivery? =
        withHandle { it.sendJoinRejection(listenerId, reason) }

    suspend fun disconnectPeer(listenerId: String, reason: String): FfiHostTransportDelivery? =
        withHandle { it.disconnectPeer(listenerId, reason) }

    suspend fun broadcastStreamStart(
        streamId: String,
        hostStartTimeMs: Long,
        sampleRate: Int,
        channels: Int,
        samplesPerPacket: Int,
        currentEntryId: String?,
        currentDisplayName: String?,
    ): FfiHostTransportDelivery? = withHandle {
        it.broadcastStreamStart(
            streamId,
            hostStartTimeMs.toULong(),
            sampleRate.toUInt(),
            channels.toUShort(),
            samplesPerPacket.toUInt(),
            currentEntryId,
            currentDisplayName,
        )
    }

    suspend fun broadcastPause(streamId: String, hostPauseTimeMs: Long): FfiHostTransportDelivery? =
        withHandle { it.broadcastPause(streamId, hostPauseTimeMs.toULong()) }

    suspend fun sendStreamStartToListener(
        listenerId: String,
        streamId: String,
        hostStartTimeMs: Long,
        sampleRate: Int,
        channels: Int,
        samplesPerPacket: Int,
        currentEntryId: String?,
        currentDisplayName: String?,
    ): FfiHostTransportDelivery? = withHandle {
        it.sendStreamStartToListener(
            listenerId,
            streamId,
            hostStartTimeMs.toULong(),
            sampleRate.toUInt(),
            channels.toUShort(),
            samplesPerPacket.toUInt(),
            currentEntryId,
            currentDisplayName,
        )
    }

    suspend fun broadcastTrackBoundary(
        streamId: String,
        firstSampleIndex: Long,
        entryId: String,
        displayName: String,
    ): FfiHostTransportDelivery? = withHandle {
        it.broadcastTrackBoundary(
            streamId,
            firstSampleIndex.toULong(),
            entryId,
            displayName,
        )
    }

    suspend fun sendTrackBoundaryToListener(
        listenerId: String,
        streamId: String,
        firstSampleIndex: Long,
        entryId: String,
        displayName: String,
    ): FfiHostTransportDelivery? = withHandle {
        it.sendTrackBoundaryToListener(
            listenerId,
            streamId,
            firstSampleIndex.toULong(),
            entryId,
            displayName,
        )
    }

    suspend fun broadcastStop(streamId: String, hostStopTimeMs: Long): FfiHostTransportDelivery? =
        withHandle { it.broadcastStop(streamId, hostStopTimeMs.toULong()) }

    suspend fun authorizeListener(listenerId: String, syncPort: UShort, audioPort: UShort) =
        withHandle { it.authorizeListener(listenerId, syncPort, audioPort) }

    suspend fun sendSyncResponse(
        correlationId: ULong,
        t1ListenerSendElapsedMs: ULong,
        t2HostReceiveElapsedMs: ULong,
        t3HostSendElapsedMs: ULong,
    ): FfiHostTransportDelivery? = withHandle {
        it.sendSyncResponse(correlationId, t1ListenerSendElapsedMs, t2HostReceiveElapsedMs, t3HostSendElapsedMs)
    }

    suspend fun broadcastAudio(
        streamId: String,
        sequence: Long,
        sampleRate: Int,
        channels: Int,
        samplesPerPacket: Int,
        firstSampleIndex: Long,
        hostPresentationTimeMs: Long,
        payload: ByteArray,
    ): FfiHostTransportDelivery? = withHandle {
        it.broadcastAudio(
            streamId,
            sequence.toULong(),
            sampleRate.toUInt(),
            channels.toUShort(),
            samplesPerPacket.toUInt(),
            firstSampleIndex.toULong(),
            hostPresentationTimeMs.toULong(),
            payload,
        )
    }

    override fun close() {
        closeExistingHandle()
    }

    private fun startEventLoop(scope: CoroutineScope, handle: FfiHostTransportHandle) {
        eventLoop?.cancel()
        eventLoop = scope.launch(Dispatchers.IO) {
            while (isActive && handleGate.currentHandle() === handle) {
                val event = try {
                    handleGate.withExpectedPoll(handle) { it.pollEvent(POLL_TIMEOUT_MS) }
                } catch (error: FfiHostTransportException) {
                    // A poll unblocked by Stop/rebind is teardown fallout, not
                    // a failure in the newly bound host session.
                    if (shouldForwardPolledHostEvent(
                            loopActive = isActive,
                            handleIsCurrent = handleGate.currentHandle() === handle,
                        )
                    ) {
                        eventQueue.emit(
                            handle,
                            FfiHostTransportEvent.PeerDisconnected(
                                listenerId = null,
                                message = error.message,
                            ),
                        )
                    }
                    break
                }
                // Native poll can return after the old session is retired.
                // Never leak its join/disconnect events into a new session.
                if (!shouldForwardPolledHostEvent(
                        loopActive = isActive,
                        handleIsCurrent = handleGate.currentHandle() === handle,
                    )
                ) {
                    break
                }
                if (event != null) {
                    when (event) {
                        is FfiHostTransportEvent.PeerAccepted -> eventQueue.emit(handle, event)
                        is FfiHostTransportEvent.Heartbeat,
                        is FfiHostTransportEvent.ResyncNotice,
                        is FfiHostTransportEvent.Rejected,
                        -> eventQueue.tryEmit(handle, event)
                        // Join, disconnect and synchronization requests must not
                        // disappear when the Android collector is temporarily slow.
                        else -> eventQueue.emit(handle, event)
                    }
                }
            }
        }
    }

    private suspend fun <T> withHandle(action: (FfiHostTransportHandle) -> T): T? {
        // Capture the session at invocation, before the dispatcher can delay
        // this operation behind a concurrent Stop/rebind. Never deliver an
        // old session's Pause/approval/audio command to its replacement.
        val expected = handleGate.currentHandle() ?: return null
        return withContext(callDispatcher) { handleGate.withExpected(expected, action) }
    }

    private fun closeExistingHandle() {
        handleGate.close { handle ->
            try {
                retireHandle(handle)
            } finally {
                eventQueue.discardRetiredEvents()
            }
        }
    }

    private fun retireHandle(handle: FfiHostTransportHandle?) {
        // Retire the poller before closing the native handle so a delayed
        // event cannot be forwarded while the next session is binding.
        eventLoop?.cancel()
        eventLoop = null
        retireNativeHandle(
            handle,
            shutdown = { it.shutdown() },
            close = { it.close() },
            awaitPoll = { handleGate.awaitPollQuiescence(it) },
        )
    }
}

/** The host event channel survives sequential sessions; retired polls must not. */
internal fun shouldForwardPolledHostEvent(
    loopActive: Boolean,
    handleIsCurrent: Boolean,
): Boolean = loopActive && handleIsCurrent
