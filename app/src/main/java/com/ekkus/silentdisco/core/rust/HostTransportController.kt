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
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private const val POLL_TIMEOUT_MS: ULong = 500uL

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
    private val eventChannel = Channel<Pair<FfiHostTransportHandle, FfiHostTransportEvent>>(
        Channel.UNLIMITED,
    )

    val events: Flow<FfiHostTransportEvent> = eventChannel.receiveAsFlow()
        .filter { (handle, _) -> handleGate.currentHandle() === handle }
        .map { (_, event) -> event }

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
                start = { handle -> startEventLoop(scope, handle) },
            )
        }
    }

    suspend fun sendJoinApproval(listenerId: String, trustedForFuture: Boolean): FfiHostTransportDelivery? =
        withHandle { it.sendJoinApproval(listenerId, trustedForFuture) }

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
                    handle.pollEvent(POLL_TIMEOUT_MS)
                } catch (error: FfiHostTransportException) {
                    // A poll unblocked by Stop/rebind is teardown fallout, not
                    // a failure in the newly bound host session.
                    if (shouldForwardPolledHostEvent(
                            loopActive = isActive,
                            handleIsCurrent = handleGate.currentHandle() === handle,
                        )
                    ) {
                        eventChannel.trySend(
                            handle to FfiHostTransportEvent.PeerDisconnected(
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
                    eventChannel.trySend(handle to event)
                }
            }
        }
    }

    private suspend fun <T> withHandle(action: (FfiHostTransportHandle) -> T): T? =
        withContext(callDispatcher) { handleGate.withCurrent(action) }

    private fun closeExistingHandle() {
        handleGate.close(::retireHandle)
    }

    private fun retireHandle(handle: FfiHostTransportHandle?) {
        // Retire the poller before closing the native handle so a delayed
        // event cannot be forwarded while the next session is binding.
        eventLoop?.cancel()
        eventLoop = null
        handle?.let {
            runCatching { it.shutdown() }
            it.close()
        }
    }
}

/** The host event channel survives sequential sessions; retired polls must not. */
internal fun shouldForwardPolledHostEvent(
    loopActive: Boolean,
    handleIsCurrent: Boolean,
): Boolean = loopActive && handleIsCurrent
