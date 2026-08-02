package com.ekkus.silentdisco.core.transport

import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.model.SessionInfo
import kotlinx.coroutines.flow.StateFlow

data class TransportPorts(
    val control: Int = 41_000,
    val sync: Int = 41_001,
    val audio: Int = 41_002,
)

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

    fun startHost(session: SessionInfo): TransportOperationResult
    fun discoverPeers()
    fun cancelDiscovery() = stop()
    fun connectToSession(session: SessionInfo)
    fun recordHeartbeat()
    fun fail(message: String, retryable: Boolean)
    fun retry()
    fun stop()
}
