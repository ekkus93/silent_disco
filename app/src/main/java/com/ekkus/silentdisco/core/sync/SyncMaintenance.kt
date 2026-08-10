package com.ekkus.silentdisco.core.sync

import android.os.SystemClock
import com.ekkus.silentdisco.core.protocol.SyncRequestPacket
import com.ekkus.silentdisco.core.protocol.SyncResponsePacket

/**
 * Simulates a host's four-timestamp response for the `BuildConfig.DEBUG`-only
 * demo-session flow (`MainViewModelSynchronization.kt`'s `requestListenerSyncProbe`
 * demo branch) -- never used against a real host, which always answers over
 * the transport instead.
 */
class HostTimingService {
    fun createResponse(request: SyncRequestPacket): SyncResponsePacket {
        val receiveTime = SystemClock.elapsedRealtime()
        val sendTime = SystemClock.elapsedRealtime()
        return SyncResponsePacket(
            version = request.version,
            sessionId = request.sessionId,
            correlationId = request.correlationId,
            t1ListenerSendElapsedMs = request.t1ListenerSendElapsedMs,
            t2HostReceiveElapsedMs = receiveTime,
            t3HostSendElapsedMs = sendTime,
        )
    }
}
