package com.ekkus.silentdisco.core.transport

import android.content.Context
import android.os.SystemClock
import com.ekkus.silentdisco.core.logging.AppLogger
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.model.SessionInfo
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

data class BleAdvertisement(
    val sessionId: String,
    val sessionName: String,
    val hostName: String,
    val approvalRequired: Boolean,
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
)

interface SessionTransport {
    val snapshot: StateFlow<TransportSnapshot>

    fun startHost(session: SessionInfo)
    fun discoverPeers()
    fun connectToSession(session: SessionInfo)
    fun recordHeartbeat()
    fun fail(message: String, retryable: Boolean)
    fun retry()
    fun stop()
}

class BleDiscoveryService(private val context: Context) {
    private val _discoveredSessions = MutableStateFlow<List<SessionInfo>>(emptyList())
    val discoveredSessions: StateFlow<List<SessionInfo>> = _discoveredSessions.asStateFlow()
    private val _advertisement = MutableStateFlow<BleAdvertisement?>(null)
    val advertisement: StateFlow<BleAdvertisement?> = _advertisement.asStateFlow()

    fun startAdvertising(advertisement: BleAdvertisement) {
        val session = SessionInfo(
            id = advertisement.sessionId,
            name = advertisement.sessionName,
            hostDeviceName = advertisement.hostName,
            approvalMode = if (advertisement.approvalRequired) {
                ApprovalMode.MANUAL
            } else {
                ApprovalMode.TRUSTED_DEVICES_PLACEHOLDER
            },
            inviteCodeRequired = false,
        )
        _advertisement.value = advertisement
        _discoveredSessions.value = listOf(session)
    }

    fun startScanning() {
        advertisement.value?.let { current ->
            _discoveredSessions.value = listOf(
                SessionInfo(
                    id = current.sessionId,
                    name = current.sessionName,
                    hostDeviceName = current.hostName,
                    approvalMode = if (current.approvalRequired) ApprovalMode.MANUAL else ApprovalMode.TRUSTED_DEVICES_PLACEHOLDER,
                    inviteCodeRequired = false,
                ),
            )
        }
    }

    fun stop() {
        _advertisement.value = null
        _discoveredSessions.value = emptyList()
    }
}

class WifiDirectTransportService(
    private val context: Context,
    private val logger: AppLogger = AppLogger(),
) : SessionTransport {
    private val _snapshot = MutableStateFlow(TransportSnapshot())
    override val snapshot: StateFlow<TransportSnapshot> = _snapshot.asStateFlow()

    override fun startHost(session: SessionInfo) {
        logger.i("transport.host", "Hosting ${session.name}")
        _snapshot.value = TransportSnapshot(
            state = TransportConnectionState.ADVERTISING,
            peers = emptyList(),
            lastContactElapsedMs = SystemClock.elapsedRealtime(),
        )
    }

    override fun discoverPeers() {
        logger.i("transport.discover", "Scanning for Wi-Fi Direct peers")
        val peers = listOf(
            WifiDirectPeer(deviceName = "Pixel Listener", deviceAddress = "02:11:22:33:44:55"),
            WifiDirectPeer(deviceName = "Galaxy Listener", deviceAddress = "AA:BB:CC:DD:EE:FF"),
        )
        _snapshot.value = _snapshot.value.copy(
            state = TransportConnectionState.DISCOVERING,
            peers = peers,
            lastError = null,
        )
    }

    override fun connectToSession(session: SessionInfo) {
        logger.i("transport.connect", "Connecting to ${session.id}")
        _snapshot.value = _snapshot.value.copy(
            state = TransportConnectionState.CONNECTED,
            peers = _snapshot.value.peers.ifEmpty {
                listOf(WifiDirectPeer(session.hostDeviceName, "HOST-${session.id.take(6)}"))
            },
            lastContactElapsedMs = SystemClock.elapsedRealtime(),
            lastError = null,
        )
    }

    override fun recordHeartbeat() {
        logger.d("transport.heartbeat", "Heartbeat recorded")
        _snapshot.value = _snapshot.value.copy(lastContactElapsedMs = SystemClock.elapsedRealtime())
    }

    override fun fail(message: String, retryable: Boolean) {
        logger.w("transport.error", message)
        _snapshot.value = _snapshot.value.copy(
            state = TransportConnectionState.FAILED,
            lastError = TransportError(message = message, retryable = retryable),
        )
    }

    override fun retry() {
        logger.i("transport.retry", "Retrying transport setup")
        _snapshot.value = _snapshot.value.copy(
            state = TransportConnectionState.RETRYING,
            retryCount = _snapshot.value.retryCount + 1,
            lastError = null,
        )
    }

    override fun stop() {
        logger.i("transport.stop", "Transport disconnected")
        _snapshot.value = TransportSnapshot(state = TransportConnectionState.DISCONNECTED)
    }
}
