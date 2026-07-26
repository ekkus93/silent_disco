package com.ekkus.silentdisco.core.transport

import android.annotation.SuppressLint
import android.content.Context
import android.net.wifi.p2p.WifiP2pManager
import android.os.Looper

class DiscoveryLifecycleController(context: Context) {
    private val appContext = context.applicationContext
    private val manager = appContext.getSystemService(Context.WIFI_P2P_SERVICE) as? WifiP2pManager
    private val channel = manager?.initialize(appContext, Looper.getMainLooper(), null)

    @SuppressLint("MissingPermission")
    fun stopDiscovery(onFailure: (String) -> Unit) {
        BleDiscoveryService.cancelActiveScan()
        val manager = manager
        val channel = channel
        if (manager == null || channel == null) {
            onFailure("Nearby-session discovery could not be stopped because Wi-Fi Direct is unavailable")
            return
        }
        manager.stopPeerDiscovery(
            channel,
            actionListener(
                operation = "stop nearby-session discovery",
                onFailure = onFailure,
            ),
        )
    }

    @SuppressLint("MissingPermission")
    fun cancelJoinTransport(onFailure: (String) -> Unit) {
        stopDiscovery(onFailure)
        val manager = manager
        val channel = channel
        if (manager == null || channel == null) {
            onFailure("The pending listener connection could not be cancelled because Wi-Fi Direct is unavailable")
            return
        }
        manager.requestConnectionInfo(channel) { info ->
            val operation = if (info.groupFormed) "disconnect from the session" else "cancel the pending connection"
            val listener = actionListener(operation, onFailure)
            if (info.groupFormed) {
                manager.removeGroup(channel, listener)
            } else {
                manager.cancelConnect(channel, listener)
            }
        }
    }

    private fun actionListener(
        operation: String,
        onFailure: (String) -> Unit,
    ): WifiP2pManager.ActionListener = object : WifiP2pManager.ActionListener {
        override fun onSuccess() = Unit

        override fun onFailure(reason: Int) {
            onFailure("Could not $operation (${reasonLabel(reason)})")
        }
    }

    private fun reasonLabel(reason: Int): String = when (reason) {
        WifiP2pManager.BUSY -> "Wi-Fi Direct is busy"
        WifiP2pManager.ERROR -> "Wi-Fi Direct reported an error"
        WifiP2pManager.P2P_UNSUPPORTED -> "Wi-Fi Direct is unsupported"
        else -> "reason $reason"
    }
}
