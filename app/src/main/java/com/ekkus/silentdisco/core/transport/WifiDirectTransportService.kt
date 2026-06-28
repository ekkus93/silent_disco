package com.ekkus.silentdisco.core.transport

import android.Manifest
import android.annotation.SuppressLint
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.net.wifi.WpsInfo
import android.net.wifi.p2p.WifiP2pConfig
import android.net.wifi.p2p.WifiP2pDevice
import android.net.wifi.p2p.WifiP2pManager
import android.os.Build
import android.os.SystemClock
import androidx.core.content.ContextCompat
import com.ekkus.silentdisco.core.logging.AppLogger
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

class WifiDirectTransportService(
    context: Context,
    private val logger: AppLogger = AppLogger(),
    private val ports: TransportPorts = TransportPorts(),
) : SessionTransport {
    private val appContext = context.applicationContext
    private val manager = appContext.getSystemService(Context.WIFI_P2P_SERVICE) as WifiP2pManager?
    private val channel = manager?.initialize(appContext, appContext.mainLooper, null)
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

    private val currentPeers = linkedMapOf<String, WifiP2pDevice>()
    private var activeSession: SessionInfo? = null
    private var pendingConnectSession: SessionInfo? = null
    private var hosting = false
    private var receiverRegistered = false

    private var controlServer: TcpServerChannel<ControlMessage>? = null
    private var syncServer: TcpServerChannel<SyncRequestPacket>? = null
    private var audioServer: TcpServerChannel<AudioPacket>? = null
    private var syncResponseServer: TcpServerChannel<SyncResponsePacket>? = null

    private var controlClient: TcpClientChannel<ControlMessage>? = null
    private var syncClient: TcpClientChannel<SyncRequestPacket>? = null
    private var syncResponseClient: TcpClientChannel<SyncResponsePacket>? = null
    private var audioClient: TcpClientChannel<AudioPacket>? = null

    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            when (intent.action) {
                WifiP2pManager.WIFI_P2P_STATE_CHANGED_ACTION -> {
                    val state = intent.getIntExtra(WifiP2pManager.EXTRA_WIFI_STATE, WifiP2pManager.WIFI_P2P_STATE_DISABLED)
                    if (state != WifiP2pManager.WIFI_P2P_STATE_ENABLED) {
                        fail("Wi-Fi Direct is disabled", retryable = true)
                    }
                }

                WifiP2pManager.WIFI_P2P_PEERS_CHANGED_ACTION -> requestPeers()
                WifiP2pManager.WIFI_P2P_CONNECTION_CHANGED_ACTION -> handleConnectionChanged(intent)
                WifiP2pManager.WIFI_P2P_DISCOVERY_CHANGED_ACTION -> {
                    recordHeartbeat()
                }
            }
        }
    }

    override fun startHost(session: SessionInfo): TransportOperationResult {
        activeSession = session
        pendingConnectSession = null
        hosting = true
        stopClientChannels()
        ensureReceiver()
        if (manager == null || channel == null) {
            val message = "Wi-Fi Direct manager unavailable on this device"
            fail(message, retryable = false)
            return TransportOperationResult.failed(message)
        }
        if (!hasWifiDirectPermission()) {
            val message = "Missing nearby Wi-Fi permission"
            fail(message, retryable = true)
            return TransportOperationResult.failed(message)
        }
        return runCatching {
            startHostSockets()
            recreateGroup()
            updateSnapshot(
                state = TransportConnectionState.ADVERTISING,
                peers = emptyList(),
                lastError = null,
                hostAddressHint = null,
            )
            TransportOperationResult.Started
        }.getOrElse { error ->
            val message = error.message ?: "Failed to start Wi-Fi Direct host"
            fail(message, retryable = true)
            TransportOperationResult.failed(message)
        }
    }

    @SuppressLint("MissingPermission")
    override fun discoverPeers() {
        ensureReceiver()
        if (manager == null || channel == null) {
            fail("Wi-Fi Direct manager unavailable on this device", retryable = false)
            return
        }
        if (!hasWifiDirectPermission()) {
            fail("Missing nearby Wi-Fi permission", retryable = true)
            return
        }
        val manager = manager ?: return
        val channel = channel ?: return
        runCatching {
            manager.discoverPeers(channel, actionListener("discover peers") {
                logger.i("transport.discover", "Started Wi-Fi Direct peer discovery")
                updateSnapshot(
                    state = TransportConnectionState.DISCOVERING,
                    peers = snapshot.value.peers,
                    lastError = null,
                    hostAddressHint = snapshot.value.hostAddressHint,
                )
                requestPeers()
            })
        }.onFailure { error ->
            fail(error.message ?: "Failed to start Wi-Fi Direct discovery", retryable = true)
        }
    }

    @SuppressLint("MissingPermission")
    override fun connectToSession(session: SessionInfo) {
        activeSession = session
        pendingConnectSession = session
        hosting = false
        stopClientChannels()
        ensureReceiver()
        if (manager == null || channel == null) {
            fail("Wi-Fi Direct manager unavailable on this device", retryable = false)
            return
        }
        if (!hasWifiDirectPermission()) {
            fail("Missing nearby Wi-Fi permission", retryable = true)
            return
        }
        val manager = manager ?: return
        val channel = channel ?: return
        val peer = resolvePeerForSession(session)
        if (peer == null) {
            fail("No matching Wi-Fi Direct peer found for ${session.hostDeviceName}", retryable = true)
            return
        }
        val config = WifiP2pConfig().apply {
            deviceAddress = peer.deviceAddress
            wps.setup = WpsInfo.PBC
            groupOwnerIntent = 0
        }
        updateSnapshot(
            state = TransportConnectionState.CONNECTING,
            peers = listOf(WifiDirectPeer(peer.deviceName, peer.deviceAddress)),
            lastError = null,
            hostAddressHint = null,
        )
        runCatching {
            manager.connect(channel, config, actionListener("connect to peer ${peer.deviceName}") {
                logger.i("transport.connect", "Connecting to Wi-Fi Direct peer ${peer.deviceName}")
            })
        }.onFailure { error ->
            fail(error.message ?: "Failed to connect to Wi-Fi Direct peer", retryable = true)
        }
    }

    override suspend fun sendControlToHost(message: ControlMessage) {
        controlClient?.send(message) ?: error("Control channel is not connected")
        recordHeartbeat()
        updateByteCounts()
    }

    override suspend fun broadcastControl(message: ControlMessage): SendAllResult {
        val result = controlServer?.sendAll(message) ?: error("Control server is not active")
        recordHeartbeat()
        updateByteCounts()
        return result
    }

    override suspend fun sendSyncRequestToHost(packet: SyncRequestPacket) {
        syncClient?.send(packet) ?: error("Sync request channel is not connected")
        recordHeartbeat()
        updateByteCounts()
    }

    override suspend fun broadcastSyncResponse(packet: SyncResponsePacket): SendAllResult {
        val result = syncResponseServer?.sendAll(packet) ?: error("Sync response server is not active")
        recordHeartbeat()
        updateByteCounts()
        return result
    }

    override suspend fun broadcastAudio(packet: AudioPacket): SendAllResult {
        val result = audioServer?.sendAll(packet) ?: error("Audio server is not active")
        recordHeartbeat()
        updateByteCounts()
        return result
    }

    override fun recordHeartbeat() {
        _snapshot.value = _snapshot.value.copy(lastContactElapsedMs = SystemClock.elapsedRealtime())
    }

    override fun fail(message: String, retryable: Boolean) {
        logger.w("transport.error", message)
        _snapshot.value = _snapshot.value.copy(
            state = TransportConnectionState.FAILED,
            lastError = TransportError(message = message, retryable = retryable),
            lastContactElapsedMs = SystemClock.elapsedRealtime(),
        )
    }

    override fun retry() {
        logger.i("transport.retry", "Retrying transport setup")
        _snapshot.value = _snapshot.value.copy(
            state = TransportConnectionState.RETRYING,
            retryCount = _snapshot.value.retryCount + 1,
            lastError = null,
        )
        when {
            hosting && activeSession != null -> startHost(activeSession!!)
            pendingConnectSession != null -> connectToSession(pendingConnectSession!!)
            else -> discoverPeers()
        }
    }

    @SuppressLint("MissingPermission")
    override fun stop() {
        logger.i("transport.stop", "Transport disconnected")
        activeSession = null
        pendingConnectSession = null
        hosting = false
        if (manager != null && channel != null && hasWifiDirectPermission()) {
            val manager = manager ?: return
            val channel = channel ?: return
            runCatching {
                manager.removeGroup(channel, null)
                manager.stopPeerDiscovery(channel, null)
            }
        }
        stopClientChannels()
        stopServerChannels()
        unregisterReceiver()
        currentPeers.clear()
        _snapshot.value = TransportSnapshot(state = TransportConnectionState.DISCONNECTED)
    }

    @SuppressLint("MissingPermission")
    private fun recreateGroup() {
        if (manager == null || channel == null) return
        if (!hasWifiDirectPermission()) {
            fail("Missing nearby Wi-Fi permission", retryable = true)
            return
        }
        val manager = manager ?: return
        val channel = channel ?: return
        runCatching {
            manager.removeGroup(channel, object : WifiP2pManager.ActionListener {
                override fun onSuccess() {
                    createGroup()
                }

                override fun onFailure(reason: Int) {
                    createGroup()
                }
            })
        }.onFailure { createGroup() }
    }

    @SuppressLint("MissingPermission")
    private fun createGroup() {
        if (manager == null || channel == null) return
        if (!hasWifiDirectPermission()) {
            fail("Missing nearby Wi-Fi permission", retryable = true)
            return
        }
        val manager = manager ?: return
        val channel = channel ?: return
        runCatching {
            manager.createGroup(channel, actionListener("create Wi-Fi Direct group") {
                logger.i("transport.host", "Wi-Fi Direct group creation requested")
            })
        }.onFailure { error ->
            fail(error.message ?: "Failed to create Wi-Fi Direct group", retryable = true)
        }
    }

    @SuppressLint("MissingPermission")
    private fun requestPeers() {
        if (manager == null || channel == null) return
        if (!hasWifiDirectPermission()) return
        val manager = manager ?: return
        val channel = channel ?: return
        runCatching {
            manager.requestPeers(channel) { peers ->
                currentPeers.clear()
                peers.deviceList.forEach { device -> currentPeers[device.deviceAddress] = device }
                updateSnapshot(
                    state = if (_snapshot.value.state == TransportConnectionState.CONNECTING) {
                        TransportConnectionState.CONNECTING
                    } else {
                        TransportConnectionState.DISCOVERING
                    },
                    peers = currentPeers.values.map { WifiDirectPeer(it.deviceName, it.deviceAddress) },
                    lastError = _snapshot.value.lastError,
                    hostAddressHint = _snapshot.value.hostAddressHint,
                )
            }
        }.onFailure { error ->
            fail(error.message ?: "Failed to request Wi-Fi Direct peers", retryable = true)
        }
    }

    private fun handleConnectionChanged(@Suppress("UNUSED_PARAMETER") intent: Intent) {
        if (manager == null || channel == null) return
        manager.requestConnectionInfo(channel) { info ->
            val hostAddress = info.groupOwnerAddress?.hostAddress ?: WIFI_DIRECT_GROUP_OWNER
            when {
                info.groupFormed && info.isGroupOwner -> {
                startHostSockets()
                updateSnapshot(
                    state = TransportConnectionState.ADVERTISING,
                    peers = currentPeers.values.map { WifiDirectPeer(it.deviceName, it.deviceAddress) },
                    lastError = null,
                    hostAddressHint = hostAddress,
                )
                logger.i("transport.host", "Group owner ready at $hostAddress")
                }
                info.groupFormed -> {
                    val session = pendingConnectSession ?: activeSession ?: return@requestConnectionInfo
                    runCatching {
                        startClientSockets(hostAddress, session)
                    }.onSuccess {
                        updateSnapshot(
                            state = TransportConnectionState.CONNECTED,
                            peers = currentPeers.values.map { WifiDirectPeer(it.deviceName, it.deviceAddress) },
                            lastError = null,
                            hostAddressHint = hostAddress,
                        )
                        logger.i("transport.connect", "Connected to group owner at $hostAddress")
                    }.onFailure { error ->
                        fail(error.message ?: "Failed to open TCP transport to host", retryable = true)
                    }
                }
                _snapshot.value.state == TransportConnectionState.CONNECTING ||
                    _snapshot.value.state == TransportConnectionState.CONNECTED -> {
                    _snapshot.value = _snapshot.value.copy(
                        state = TransportConnectionState.DISCONNECTED,
                        lastContactElapsedMs = SystemClock.elapsedRealtime(),
                    )
                }
            }
        }
    }

    private fun startHostSockets() {
        if (controlServer != null) return
        controlServer = TcpServerChannel(
            port = ports.control,
            channelName = "control",
            codec = ControlMessageCodec,
            logger = logger,
        ).also {
            it.start()
            observeControlServer(it)
        }
        syncServer = TcpServerChannel(
            port = ports.sync,
            channelName = "sync-request",
            codec = SyncRequestPacketCodec,
            logger = logger,
        ).also {
            it.start()
            observeSyncServer(it)
        }
        syncResponseServer = TcpServerChannel(
            port = ports.sync + 100,
            channelName = "sync-response",
            codec = SyncResponsePacketCodec,
            logger = logger,
        ).also {
            it.start()
            observeSyncResponseServer(it)
        }
        audioServer = TcpServerChannel(
            port = ports.audio,
            channelName = "audio",
            codec = AudioPacketCodec,
            logger = logger,
        ).also {
            it.start()
            observeAudioServer(it)
        }
    }

    private fun startClientSockets(hostAddress: String, session: SessionInfo) {
        if (controlClient?.isConnected() == true && snapshot.value.hostAddressHint == hostAddress) return
        stopClientChannels()
        activeSession = session
        controlClient = TcpClientChannel(
            host = hostAddress,
            port = ports.control,
            channelName = "control",
            codec = ControlMessageCodec,
            logger = logger,
        ).also {
            it.connect()
            observeControlClient(it)
        }
        syncClient = TcpClientChannel(
            host = hostAddress,
            port = ports.sync,
            channelName = "sync-request",
            codec = SyncRequestPacketCodec,
            logger = logger,
        ).also { it.connect() }
        syncResponseClient = TcpClientChannel(
            host = hostAddress,
            port = ports.sync + 100,
            channelName = "sync-response",
            codec = SyncResponsePacketCodec,
            logger = logger,
        ).also {
            it.connect()
            observeSyncResponseClient(it)
        }
        audioClient = TcpClientChannel(
            host = hostAddress,
            port = ports.audio,
            channelName = "audio",
            codec = AudioPacketCodec,
            logger = logger,
        ).also {
            it.connect()
            observeAudioClient(it)
        }
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
        state: TransportConnectionState,
        peers: List<WifiDirectPeer>,
        lastError: TransportError?,
        hostAddressHint: String?,
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
            lastContactElapsedMs = SystemClock.elapsedRealtime(),
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

    private fun resolvePeerForSession(session: SessionInfo): WifiP2pDevice? =
        currentPeers.values.firstOrNull { it.deviceName == session.hostDeviceName } ?: currentPeers.values.firstOrNull()

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

    private fun ensureReceiver() {
        if (receiverRegistered) return
        val filter = IntentFilter().apply {
            addAction(WifiP2pManager.WIFI_P2P_STATE_CHANGED_ACTION)
            addAction(WifiP2pManager.WIFI_P2P_PEERS_CHANGED_ACTION)
            addAction(WifiP2pManager.WIFI_P2P_CONNECTION_CHANGED_ACTION)
            addAction(WifiP2pManager.WIFI_P2P_DISCOVERY_CHANGED_ACTION)
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            appContext.registerReceiver(receiver, filter, Context.RECEIVER_NOT_EXPORTED)
        } else {
            @Suppress("DEPRECATION")
            appContext.registerReceiver(receiver, filter)
        }
        receiverRegistered = true
    }

    private fun unregisterReceiver() {
        if (!receiverRegistered) return
        runCatching { appContext.unregisterReceiver(receiver) }
        receiverRegistered = false
    }

    private fun actionListener(action: String, onSuccess: () -> Unit): WifiP2pManager.ActionListener =
        object : WifiP2pManager.ActionListener {
            override fun onSuccess() = onSuccess()

            override fun onFailure(reason: Int) {
                fail("Wi-Fi Direct failed to $action (reason=$reason)", retryable = true)
            }
        }

    private fun hasWifiDirectPermission(): Boolean {
        val permission = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            Manifest.permission.NEARBY_WIFI_DEVICES
        } else {
            Manifest.permission.ACCESS_FINE_LOCATION
        }
        return ContextCompat.checkSelfPermission(appContext, permission) == PackageManager.PERMISSION_GRANTED
    }

    private companion object {
        const val WIFI_DIRECT_GROUP_OWNER = "192.168.49.1"
    }
}
