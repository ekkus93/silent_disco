package com.ekkus.silentdisco.app

import android.os.SystemClock
import com.ekkus.silentdisco.core.rust.HostCoreController
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportHandle
import com.ekkus.silentdisco.core.uniffi.FfiJoinRequestInput
import com.ekkus.silentdisco.core.uniffi.FfiTrustState

/** Native events are projected into the Rust actor with their source generation. */
internal fun MainViewModel.handleHostTransportEvent(
    controller: HostCoreController,
    event: FfiHostTransportEvent,
    sourceHandle: FfiHostTransportHandle,
) {
    when (event) {
        is FfiHostTransportEvent.JoinRequestReceived -> {
            val requestId = newHostJoinRequestId()
            val tracked = pendingListenerDatagramPorts.record(
                requestId, event.deviceId, event.syncPort, event.audioPort, sourceHandle,
            )
            if (!tracked) {
                // Bounded route tracking must fail closed rather than silently
                // losing the original TCP generation and approving another one.
                reportRustHostCommandFailure(
                    "track listener join",
                    IllegalStateException("Pending listener route capacity exceeded"),
                )
            }
            runCatching {
                controller.submitJoinRequest(
                    FfiJoinRequestInput(
                        requestId = requestId,
                        deviceId = event.deviceId,
                        displayName = event.displayName,
                        trustState = FfiTrustState.UNKNOWN,
                        inviteCode = event.inviteCode,
                        receivedAtMs = SystemClock.elapsedRealtime().toULong(),
                    ),
                )
            }.onFailure { error ->
                pendingListenerDatagramPorts.discard(requestId, event.deviceId)
                reportRustHostCommandFailure("submit join request", error)
            }
        }
        is FfiHostTransportEvent.PeerDisconnected -> event.listenerId?.let { listenerId ->
            // A pending request's datagram ports are invalid as soon as its
            // claimed device ID disconnects, even if the actor's approval
            // effect was queued before this terminal event.
            pendingListenerDatagramPorts.removeDevice(listenerId)
            runCatching { controller.submitListenerDisconnected(listenerId) }
                .onFailure { error -> reportRustHostCommandFailure("submit listener disconnected", error) }
        }
        is FfiHostTransportEvent.SyncRequestReceived -> respondToSyncRequest(event, sourceHandle)
        is FfiHostTransportEvent.PeerAccepted,
        is FfiHostTransportEvent.Heartbeat,
        is FfiHostTransportEvent.ResyncNotice,
        is FfiHostTransportEvent.Rejected,
        -> Unit // diagnostic-only for this pass; resync notices are out of scope until Block 23
    }
}
