package com.ekkus.silentdisco.app

import android.os.SystemClock
import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportHandle
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

/**
 * Keep the generation captured by the native event queue across IO dispatch.
 * A delayed old-session SyncRequest cannot send into a newly rebound host.
 */
internal fun MainViewModel.respondToSyncRequest(
    event: FfiHostTransportEvent.SyncRequestReceived,
    sourceHandle: FfiHostTransportHandle,
) {
    val t2HostReceiveElapsedMs = SystemClock.elapsedRealtime().toULong()
    val admitted = hostRecoveryDispatch.launchSync(viewModelScope, Dispatchers.IO) {
        try {
            val listenerId = checkNotNull(event.listenerId) {
                "Clock synchronization request had no authorized listener identity"
            }
            hostTransportController.sendSyncResponseToListenerForGeneration(
                expected = sourceHandle,
                listenerId = listenerId,
                correlationId = event.correlationId,
                t1ListenerSendElapsedMs = event.t1ListenerSendElapsedMs,
                t2HostReceiveElapsedMs = t2HostReceiveElapsedMs,
            )
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (error: Exception) {
            reportRustHostCommandFailure("send sync response", error)
        }
    }
    if (!admitted) {
        // Clock probes are retriable; refuse an unbounded backlog of IO
        // continuations and surface a sampled error rather than flooding UI.
        val rejected = hostRecoveryDispatch.rejectedSyncRequests
        if (rejected == 1L || rejected % 64L == 0L) {
            reportRustHostCommandFailure(
                "send sync response",
                IllegalStateException("Host clock-sync delivery saturated ($rejected refusals)"),
            )
        }
    }
}
