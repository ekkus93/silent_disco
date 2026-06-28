package com.ekkus.silentdisco.core.transport

import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.ControlMessage
import com.ekkus.silentdisco.core.protocol.SyncRequestPacket
import com.ekkus.silentdisco.core.protocol.SyncResponsePacket
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow

data class BleAdvertisement(
    val sessionId: String,
    val sessionName: String,
    val hostName: String,
    val approvalRequired: Boolean,
    val inviteCodeRequired: Boolean = false,
)

data class WifiDirectPeer(
    val deviceName: String,
    val deviceAddress: String,
)

data class TransportError(
    val message: String,
    val retryable: Boolean,
)

data class TransportSnapshot(
    val state: TransportConnectionState = TransportConnectionState.IDLE,
    val peers: List<WifiDirectPeer> = emptyList(),
    val lastError: TransportError? = null,
    val lastContactElapsedMs: Long? = null,
    val retryCount: Int = 0,
    val controlConnections: Int = 0,
    val syncConnections: Int = 0,
    val audioConnections: Int = 0,
    val bytesSent: Long = 0,
    val bytesReceived: Long = 0,
    val hostAddressHint: String? = null,
)

data class TransportOperationResult(
    val started: Boolean,
    val message: String? = null,
) {
    companion object {
        val Started = TransportOperationResult(started = true)
        fun failed(message: String) = TransportOperationResult(started = false, message = message)
    }
}

interface SessionTransport {
    val snapshot: StateFlow<TransportSnapshot>
    val controlMessages: SharedFlow<ControlMessage>
    val syncRequests: SharedFlow<SyncRequestPacket>
    val syncResponses: SharedFlow<SyncResponsePacket>
    val audioPackets: SharedFlow<AudioPacket>

    fun startHost(session: SessionInfo): TransportOperationResult
    fun discoverPeers()
    fun connectToSession(session: SessionInfo)
    suspend fun sendControlToHost(message: ControlMessage)
    suspend fun broadcastControl(message: ControlMessage)
    suspend fun sendSyncRequestToHost(packet: SyncRequestPacket)
    suspend fun broadcastSyncResponse(packet: SyncResponsePacket)
    suspend fun broadcastAudio(packet: AudioPacket)
    fun recordHeartbeat()
    fun fail(message: String, retryable: Boolean)
    fun retry()
    fun stop()
}
