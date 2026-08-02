package com.ekkus.silentdisco.app

import android.app.Application
import android.content.Context
import android.net.wifi.WifiManager
import com.ekkus.silentdisco.core.rust.NetworkSessionLock

/**
 * Holds the Wi-Fi radio in low-latency mode while a manual-connect session is
 * live, preventing Android Wi-Fi power save from buffering inbound audio and
 * sync packets at the access point (observed on a real device as multi-second
 * mid-stream arrival outages and connection-start sync RTTs in the hundreds
 * of milliseconds to seconds). Reference counting is disabled so acquire and
 * release stay idempotent, matching the [NetworkSessionLock] contract.
 */
class WifiLowLatencyNetworkLock(application: Application) : NetworkSessionLock {
    private val lock = (application.getSystemService(Context.WIFI_SERVICE) as WifiManager)
        .createWifiLock(WifiManager.WIFI_MODE_FULL_LOW_LATENCY, "silent-disco:manual-listener")
        .apply { setReferenceCounted(false) }

    override fun acquire() {
        if (!lock.isHeld) lock.acquire()
    }

    override fun release() {
        if (lock.isHeld) lock.release()
    }
}
