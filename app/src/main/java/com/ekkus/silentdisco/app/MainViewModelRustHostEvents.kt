package com.ekkus.silentdisco.app

import android.os.SystemClock
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
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
                controlAddress = event.controlAddress,
            )
            if (!tracked) {
                // Do not enqueue an unapprovable actor request. Attempt a
                // generation-bound busy refusal on the original TCP session
                // instead of silently dropping this accepted join under load.
                reportRustHostCommandFailure(
                    "track listener join",
                    IllegalStateException("Pending listener route capacity exceeded"),
                )
                refuseUnprojectedHostJoin(event.deviceId, event.controlAddress, sourceHandle)
                return
            }
            runCatching {
                controller.submitJoinRequest(
                    FfiJoinRequestInput(
                        requestId = requestId,
                        deviceId = event.deviceId,
                        displayName = event.displayName,
                        inviteCode = event.inviteCode,
                        receivedAtMs = SystemClock.elapsedRealtime().toULong(),
                    ),
                )
            }.onFailure { error ->
                pendingListenerDatagramPorts.discard(requestId, event.deviceId)
                reportRustHostCommandFailure("submit join request", error)
                // The TCP reader already accepted this join. If the Rust
                // actor cannot ingest it, leaving the peer pending would
                // strand its approval spinner indefinitely.
                refuseUnprojectedHostJoin(event.deviceId, event.controlAddress, sourceHandle)
            }
        }
        is FfiHostTransportEvent.PeerDisconnectedAt -> event.listenerId?.let { listenerId ->
            // TCP source address identifies the retired peer even when a
            // replacement join claims the same unverified legacy device ID.
            pendingListenerDatagramPorts.removePeer(listenerId, event.controlAddress)
            runCatching { controller.submitListenerDisconnected(listenerId) }
                .onFailure { error -> reportRustHostCommandFailure("submit listener disconnected", error) }
        }
        is FfiHostTransportEvent.PeerDisconnected -> event.listenerId?.let { listenerId ->
            // Address-less synthetic poll failures cannot identify one TCP
            // generation; fail closed rather than reusing stale routes.
            pendingListenerDatagramPorts.removeDevice(listenerId)
            runCatching { controller.submitListenerDisconnected(listenerId) }
                .onFailure { error -> reportRustHostCommandFailure("submit listener disconnected", error) }
        }
        is FfiHostTransportEvent.SyncRequestReceived -> respondToSyncRequest(event, sourceHandle)
        is FfiHostTransportEvent.ResyncNotice -> replayHostControlsForResync(event, sourceHandle)
        is FfiHostTransportEvent.PeerAccepted,
        is FfiHostTransportEvent.Heartbeat,
        is FfiHostTransportEvent.Rejected,
        -> Unit // best-effort connection diagnostics
    }
}

/**
 * A join that cannot enter the authoritative actor must receive a visible
 * native refusal, not remain pending forever. Bind the send to the event's
 * original host generation; a late IO dispatch cannot reject a replacement
 * session. Do not treat a missing handle or zero successful peers as delivery.
 */
private fun MainViewModel.refuseUnprojectedHostJoin(
    deviceId: String,
    controlAddress: String,
    sourceHandle: FfiHostTransportHandle,
) {
    viewModelScope.launch(Dispatchers.IO) {
        try {
            val delivery = hostTransportController.sendJoinRejectionForGeneration(
                sourceHandle, deviceId, controlAddress, "host_busy",
            )
            check(delivery != null && delivery.successfulPeers > 0u) {
                "Host join refusal was not delivered to the original listener"
            }
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (error: Exception) {
            reportRustHostCommandFailure("refuse unavailable join", error)
        }
    }
}
