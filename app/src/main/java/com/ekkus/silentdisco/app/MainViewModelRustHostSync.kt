package com.ekkus.silentdisco.app

import android.os.SystemClock
import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportHandle
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
    viewModelScope.launch(Dispatchers.IO) {
        val t3HostSendElapsedMs = SystemClock.elapsedRealtime().toULong()
        runCatching {
            hostTransportController.sendSyncResponseForGeneration(
                expected = sourceHandle,
                correlationId = event.correlationId,
                t1ListenerSendElapsedMs = event.t1ListenerSendElapsedMs,
                t2HostReceiveElapsedMs = t2HostReceiveElapsedMs,
                t3HostSendElapsedMs = t3HostSendElapsedMs,
            )
        }.onFailure { error -> reportRustHostCommandFailure("send sync response", error) }
    }
}
