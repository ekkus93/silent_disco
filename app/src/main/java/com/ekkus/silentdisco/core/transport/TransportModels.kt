package com.ekkus.silentdisco.core.transport

import android.content.Context
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

class BleDiscoveryService(private val context: Context) {
    private val _discoveredSessions = MutableStateFlow<List<SessionInfo>>(emptyList())
    val discoveredSessions: StateFlow<List<SessionInfo>> = _discoveredSessions.asStateFlow()

    fun startAdvertising(advertisement: BleAdvertisement) {
        val session = SessionInfo(
            id = advertisement.sessionId,
            name = advertisement.sessionName,
            hostDeviceName = advertisement.hostName,
            approvalMode = if (advertisement.approvalRequired) {
                com.ekkus.silentdisco.core.model.ApprovalMode.MANUAL
            } else {
                com.ekkus.silentdisco.core.model.ApprovalMode.TRUSTED_DEVICES_PLACEHOLDER
            },
            inviteCodeRequired = false,
        )
        _discoveredSessions.value = listOf(session)
    }

    fun startScanning() {
        // Real BLE scanner wiring belongs behind this boundary.
    }

    fun stop() {
        _discoveredSessions.value = emptyList()
    }
}

class WifiDirectTransportService(private val context: Context) {
    private val _peers = MutableStateFlow<List<WifiDirectPeer>>(emptyList())
    val peers: StateFlow<List<WifiDirectPeer>> = _peers.asStateFlow()

    fun startHost() {
        _peers.value = emptyList()
    }

    fun discoverPeers() {
        // Real WifiP2pManager integration belongs here.
    }

    fun stop() {
        _peers.value = emptyList()
    }
}
