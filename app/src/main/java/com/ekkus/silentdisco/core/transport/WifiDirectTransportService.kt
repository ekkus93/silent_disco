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
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

class WifiDirectTransportService(
    context: Context,
    private val logger: AppLogger = AppLogger(),
) : SessionTransport {
    private val appContext = context.applicationContext
    private val manager = appContext.getSystemService(Context.WIFI_P2P_SERVICE) as WifiP2pManager?
    private val channel = manager?.initialize(appContext, appContext.mainLooper, null)

    private val _snapshot = MutableStateFlow(TransportSnapshot())
    override val snapshot: StateFlow<TransportSnapshot> = _snapshot.asStateFlow()

    private val currentPeers = linkedMapOf<String, WifiP2pDevice>()
    private var activeSession: SessionInfo? = null
    private var pendingConnectSession: SessionInfo? = null
    private var hosting = false
    private var receiverRegistered = false

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
                updateSnapshot(
                    state = TransportConnectionState.ADVERTISING,
                    peers = currentPeers.values.map { WifiDirectPeer(it.deviceName, it.deviceAddress) },
                    lastError = null,
                    hostAddressHint = hostAddress,
                )
                logger.i("transport.host", "Group owner ready at $hostAddress")
                }
                info.groupFormed -> {
                    updateSnapshot(
                        state = TransportConnectionState.CONNECTED,
                        peers = currentPeers.values.map { WifiDirectPeer(it.deviceName, it.deviceAddress) },
                        lastError = null,
                        hostAddressHint = hostAddress,
                    )
                    logger.i("transport.connect", "Connected to group owner at $hostAddress")
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
        )
    }

    private fun resolvePeerForSession(session: SessionInfo): WifiP2pDevice? =
        currentPeers.values.firstOrNull { it.deviceName == session.hostDeviceName } ?: currentPeers.values.firstOrNull()

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
