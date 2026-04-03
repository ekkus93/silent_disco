package com.ekkus.silentdisco.core.transport

import android.content.Context
import android.os.SystemClock
import com.ekkus.silentdisco.core.logging.AppLogger
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.ControlMessage
import com.ekkus.silentdisco.core.protocol.SyncRequestPacket
import com.ekkus.silentdisco.core.protocol.SyncResponsePacket
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

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
    val controlConnections: Int = 0,
    val syncConnections: Int = 0,
    val audioConnections: Int = 0,
    val bytesSent: Long = 0,
    val bytesReceived: Long = 0,
    val hostAddressHint: String? = null,
)

interface SessionTransport {
    val snapshot: StateFlow<TransportSnapshot>
    val controlMessages: SharedFlow<ControlMessage>
    val syncRequests: SharedFlow<SyncRequestPacket>
    val syncResponses: SharedFlow<SyncResponsePacket>
    val audioPackets: SharedFlow<AudioPacket>

    fun startHost(session: SessionInfo)
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
    private val ports: TransportPorts = TransportPorts(),
) : SessionTransport {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val _snapshot = MutableStateFlow(TransportSnapshot())
    override val snapshot: StateFlow<TransportSnapshot> = _snapshot.asStateFlow()

    private val _controlMessages = MutableSharedFlow<ControlMessage>(extraBufferCapacity = 64)
    override val controlMessages: SharedFlow<ControlMessage> = _controlMessages.asSharedFlow()
    private val _syncRequests = MutableSharedFlow<SyncRequestPacket>(extraBufferCapacity = 64)
    override val syncRequests: SharedFlow<SyncRequestPacket> = _syncRequests.asSharedFlow()
    private val _syncResponses = MutableSharedFlow<SyncResponsePacket>(extraBufferCapacity = 64)
    override val syncResponses: SharedFlow<SyncResponsePacket> = _syncResponses.asSharedFlow()
    private val _audioPackets = MutableSharedFlow<AudioPacket>(extraBufferCapacity = 256)
    override val audioPackets: SharedFlow<AudioPacket> = _audioPackets.asSharedFlow()

    private var controlServer: TcpServerChannel<ControlMessage>? = null
    private var syncServer: TcpServerChannel<SyncRequestPacket>? = null
    private var audioServer: TcpServerChannel<AudioPacket>? = null
    private var syncResponseServer: TcpServerChannel<SyncResponsePacket>? = null

    private var controlClient: TcpClientChannel<ControlMessage>? = null
    private var syncClient: TcpClientChannel<SyncRequestPacket>? = null
    private var syncResponseClient: TcpClientChannel<SyncResponsePacket>? = null
    private var audioClient: TcpClientChannel<AudioPacket>? = null
    private var activeSession: SessionInfo? = null

    override fun startHost(session: SessionInfo) {
        activeSession = session
        stopClientChannels()
        stopServerChannels()
        try {
            controlServer = TcpServerChannel(
                port = ports.control,
                channelName = "control",
                codec = ControlMessageCodec,
                logger = logger,
            ).also { server ->
                server.start()
                observeControlServer(server)
            }
            syncServer = TcpServerChannel(
                port = ports.sync,
                channelName = "sync-request",
                codec = SyncRequestPacketCodec,
                logger = logger,
            ).also { server ->
                server.start()
                observeSyncServer(server)
            }
            syncResponseServer = TcpServerChannel(
                port = ports.sync + 100,
                channelName = "sync-response",
                codec = SyncResponsePacketCodec,
                logger = logger,
            ).also { server ->
                server.start()
                observeSyncResponseServer(server)
            }
            audioServer = TcpServerChannel(
                port = ports.audio,
                channelName = "audio",
                codec = AudioPacketCodec,
                logger = logger,
            ).also { server ->
                server.start()
                observeAudioServer(server)
            }
            logger.i("transport.host", "Hosting ${session.name} on TCP ports ${ports.control}/${ports.sync}/${ports.audio}")
            updateSnapshot(
                state = TransportConnectionState.ADVERTISING,
                peers = emptyList(),
                lastError = null,
                hostAddressHint = LOOPBACK_HOST,
            )
        } catch (error: Exception) {
            logger.e("transport.host", "Failed to start TCP host", error)
            fail(error.message ?: "Failed to bind host sockets", retryable = true)
        }
    }

    override fun discoverPeers() {
        logger.i("transport.discover", "Scanning for Wi-Fi Direct peers")
        val advertisedSession = activeSession
        val peers = buildList {
            advertisedSession?.let { add(WifiDirectPeer(deviceName = it.hostDeviceName, deviceAddress = LOOPBACK_HOST)) }
            add(WifiDirectPeer(deviceName = "Wi-Fi Direct Group Owner", deviceAddress = WIFI_DIRECT_GROUP_OWNER))
        }.distinctBy { it.deviceAddress }
        updateSnapshot(
            state = TransportConnectionState.DISCOVERING,
            peers = peers,
            lastError = null,
            hostAddressHint = peers.firstOrNull()?.deviceAddress,
        )
    }

    override fun connectToSession(session: SessionInfo) {
        activeSession = session
        stopClientChannels()
        val hostAddress = resolveHostAddress(session)
        try {
            controlClient = TcpClientChannel(
                host = hostAddress,
                port = ports.control,
                channelName = "control",
                codec = ControlMessageCodec,
                logger = logger,
            ).also { client ->
                client.connect()
                observeControlClient(client)
            }
            syncClient = TcpClientChannel(
                host = hostAddress,
                port = ports.sync,
                channelName = "sync-request",
                codec = SyncRequestPacketCodec,
                logger = logger,
            ).also { client ->
                client.connect()
            }
            syncResponseClient = TcpClientChannel(
                host = hostAddress,
                port = ports.sync + 100,
                channelName = "sync-response",
                codec = SyncResponsePacketCodec,
                logger = logger,
            ).also { client ->
                client.connect()
                observeSyncResponseClient(client)
            }
            audioClient = TcpClientChannel(
                host = hostAddress,
                port = ports.audio,
                channelName = "audio",
                codec = AudioPacketCodec,
                logger = logger,
            ).also { client ->
                client.connect()
                observeAudioClient(client)
            }
            logger.i("transport.connect", "Connected TCP client to ${session.id} at $hostAddress")
            updateSnapshot(
                state = TransportConnectionState.CONNECTED,
                peers = listOf(WifiDirectPeer(session.hostDeviceName, hostAddress)),
                lastError = null,
                hostAddressHint = hostAddress,
            )
        } catch (error: Exception) {
            logger.e("transport.connect", "TCP client connect failed", error)
            fail(error.message ?: "Failed to connect to host transport", retryable = true)
        }
    }

    override suspend fun sendControlToHost(message: ControlMessage) {
        controlClient?.send(message) ?: error("Control channel is not connected")
        recordHeartbeat()
    }

    override suspend fun broadcastControl(message: ControlMessage) {
        controlServer?.sendAll(message) ?: error("Control server is not active")
        recordHeartbeat()
    }

    override suspend fun sendSyncRequestToHost(packet: SyncRequestPacket) {
        syncClient?.send(packet) ?: error("Sync request channel is not connected")
        recordHeartbeat()
    }

    override suspend fun broadcastSyncResponse(packet: SyncResponsePacket) {
        syncResponseServer?.sendAll(packet) ?: error("Sync response server is not active")
        recordHeartbeat()
    }

    override suspend fun broadcastAudio(packet: AudioPacket) {
        audioServer?.sendAll(packet) ?: error("Audio server is not active")
        recordHeartbeat()
    }

    override fun recordHeartbeat() {
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
        activeSession = null
        stopClientChannels()
        stopServerChannels()
        _snapshot.value = TransportSnapshot(state = TransportConnectionState.DISCONNECTED)
    }

    private fun observeControlServer(server: TcpServerChannel<ControlMessage>) {
        server.incoming.collectInto(_controlMessages::tryEmit)
    }

    private fun observeSyncServer(server: TcpServerChannel<SyncRequestPacket>) {
        server.incoming.collectInto(_syncRequests::tryEmit)
    }

    private fun observeSyncResponseServer(server: TcpServerChannel<SyncResponsePacket>) {
        server.incoming.collectInto(_syncResponses::tryEmit)
    }

    private fun observeAudioServer(server: TcpServerChannel<AudioPacket>) {
        server.incoming.collectInto(_audioPackets::tryEmit)
    }

    private fun observeControlClient(client: TcpClientChannel<ControlMessage>) {
        client.incoming.collectInto(_controlMessages::tryEmit)
    }

    private fun observeSyncResponseClient(client: TcpClientChannel<SyncResponsePacket>) {
        client.incoming.collectInto(_syncResponses::tryEmit)
    }

    private fun observeAudioClient(client: TcpClientChannel<AudioPacket>) {
        client.incoming.collectInto(_audioPackets::tryEmit)
    }

    private fun <T> SharedFlow<TransportEvent<T>>.collectInto(emit: (T) -> Boolean) {
        scope.launch {
            collect { event ->
                emit(event.message)
                recordHeartbeat()
                updateByteCounts()
                logger.d("transport.message", "Received message from ${event.remoteAddress}")
            }
        }
    }

    private fun updateSnapshot(
        state: TransportConnectionState = _snapshot.value.state,
        peers: List<WifiDirectPeer> = _snapshot.value.peers,
        lastError: TransportError? = _snapshot.value.lastError,
        hostAddressHint: String? = _snapshot.value.hostAddressHint,
    ) {
        _snapshot.value = _snapshot.value.copy(
            state = state,
            peers = peers,
            lastError = lastError,
            lastContactElapsedMs = SystemClock.elapsedRealtime(),
            hostAddressHint = hostAddressHint,
            controlConnections = controlServer?.connectionCount() ?: if (controlClient?.isConnected() == true) 1 else 0,
            syncConnections = (syncServer?.connectionCount() ?: 0) +
                (syncResponseServer?.connectionCount() ?: 0) +
                if (syncClient?.isConnected() == true || syncResponseClient?.isConnected() == true) 1 else 0,
            audioConnections = audioServer?.connectionCount() ?: if (audioClient?.isConnected() == true) 1 else 0,
            bytesSent = currentBytesSent(),
            bytesReceived = currentBytesReceived(),
        )
    }

    private fun updateByteCounts() {
        _snapshot.value = _snapshot.value.copy(
            bytesSent = currentBytesSent(),
            bytesReceived = currentBytesReceived(),
            controlConnections = controlServer?.connectionCount() ?: if (controlClient?.isConnected() == true) 1 else 0,
            syncConnections = (syncServer?.connectionCount() ?: 0) +
                (syncResponseServer?.connectionCount() ?: 0) +
                if (syncClient?.isConnected() == true || syncResponseClient?.isConnected() == true) 1 else 0,
            audioConnections = audioServer?.connectionCount() ?: if (audioClient?.isConnected() == true) 1 else 0,
        )
    }

    private fun currentBytesSent(): Long =
        (controlServer?.bytesSent() ?: 0L) +
            (syncServer?.bytesSent() ?: 0L) +
            (syncResponseServer?.bytesSent() ?: 0L) +
            (audioServer?.bytesSent() ?: 0L) +
            (controlClient?.bytesSent() ?: 0L) +
            (syncClient?.bytesSent() ?: 0L) +
            (syncResponseClient?.bytesSent() ?: 0L) +
            (audioClient?.bytesSent() ?: 0L)

    private fun currentBytesReceived(): Long =
        (controlServer?.bytesReceived() ?: 0L) +
            (syncServer?.bytesReceived() ?: 0L) +
            (syncResponseServer?.bytesReceived() ?: 0L) +
            (audioServer?.bytesReceived() ?: 0L) +
            (controlClient?.bytesReceived() ?: 0L) +
            (syncClient?.bytesReceived() ?: 0L) +
            (syncResponseClient?.bytesReceived() ?: 0L) +
            (audioClient?.bytesReceived() ?: 0L)

    private fun resolveHostAddress(session: SessionInfo): String =
        if (controlServer != null && activeSession?.id == session.id) LOOPBACK_HOST
        else _snapshot.value.hostAddressHint ?: WIFI_DIRECT_GROUP_OWNER

    private fun stopClientChannels() {
        controlClient?.close()
        controlClient = null
        syncClient?.close()
        syncClient = null
        syncResponseClient?.close()
        syncResponseClient = null
        audioClient?.close()
        audioClient = null
    }

    private fun stopServerChannels() {
        controlServer?.close()
        controlServer = null
        syncServer?.close()
        syncServer = null
        syncResponseServer?.close()
        syncResponseServer = null
        audioServer?.close()
        audioServer = null
    }

    private companion object {
        const val LOOPBACK_HOST = "127.0.0.1"
        const val WIFI_DIRECT_GROUP_OWNER = "192.168.49.1"
    }
}
