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

data class SendAllResult(
    val peerCount: Int,
    val successCount: Int,
    val failureCount: Int,
) {
    val deliveredToAnyPeer: Boolean get() = successCount > 0
    val allDelivered: Boolean get() = peerCount > 0 && failureCount == 0
}


data class TargetedDeliveryResult(
    val listenerId: String,
    val intendedPeerCount: Int,
    val successCount: Int,
    val failureCount: Int,
    val message: String? = null,
) {
    val deliveredToTarget: Boolean
        get() = intendedPeerCount == 1 && successCount == 1 && failureCount == 0

    companion object {
        fun delivered(listenerId: String) = TargetedDeliveryResult(
            listenerId = listenerId,
            intendedPeerCount = 1,
            successCount = 1,
            failureCount = 0,
        )

        fun notFound(listenerId: String, message: String) = TargetedDeliveryResult(
            listenerId = listenerId,
            intendedPeerCount = 0,
            successCount = 0,
            failureCount = 0,
            message = message,
        )

        fun failed(listenerId: String, message: String) = TargetedDeliveryResult(
            listenerId = listenerId,
            intendedPeerCount = 1,
            successCount = 0,
            failureCount = 1,
            message = message,
        )
    }
}

enum class BroadcastDeliverySeverity {
    OK,
    ZERO_PEERS,
    PARTIAL_FAILURE,
}

data class BroadcastDeliveryReport(
    val severity: BroadcastDeliverySeverity,
    val message: String?,
)

fun classifyBroadcastDelivery(action: String, result: SendAllResult): BroadcastDeliveryReport = when {
    result.peerCount == 0 -> BroadcastDeliveryReport(
        BroadcastDeliverySeverity.ZERO_PEERS,
        "$action was not delivered: no connected listeners",
    )
    result.failureCount > 0 -> BroadcastDeliveryReport(
        BroadcastDeliverySeverity.PARTIAL_FAILURE,
        "$action delivered to ${result.successCount}/${result.peerCount} listeners; ${result.failureCount} failed",
    )
    else -> BroadcastDeliveryReport(BroadcastDeliverySeverity.OK, null)
}

interface SessionTransport {
    val snapshot: StateFlow<TransportSnapshot>
    val controlMessages: SharedFlow<ControlMessage>
    val syncRequests: SharedFlow<SyncRequestPacket>
    val syncResponses: SharedFlow<SyncResponsePacket>
    val audioPackets: SharedFlow<AudioPacket>

    fun startHost(session: SessionInfo): TransportOperationResult
    fun discoverPeers()
    fun cancelDiscovery() = stop()
    fun connectToSession(session: SessionInfo)
    suspend fun sendControlToHost(message: ControlMessage)
    suspend fun sendControlToListener(
        listenerId: String,
        message: ControlMessage,
    ): TargetedDeliveryResult
    suspend fun broadcastControl(message: ControlMessage): SendAllResult
    suspend fun sendSyncRequestToHost(packet: SyncRequestPacket)
    suspend fun broadcastSyncResponse(packet: SyncResponsePacket): SendAllResult
    suspend fun broadcastAudio(packet: AudioPacket): SendAllResult
    fun recordHeartbeat()
    fun fail(message: String, retryable: Boolean)
    fun retry()
    fun stop()
}
