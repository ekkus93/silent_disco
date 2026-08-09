package com.ekkus.silentdisco.app

import android.app.Application
import android.content.Context
import android.net.wifi.WifiManager
import android.os.Build
import com.ekkus.silentdisco.core.rust.NetworkSessionLock

/**
 * Holds the Wi-Fi radio in low-latency mode while a manual-connect session is
 * live, preventing Android Wi-Fi power save from buffering inbound audio and
 * sync packets at the access point (observed on a real device as multi-second
 * mid-stream arrival outages and connection-start sync RTTs in the hundreds
 * of milliseconds to seconds). Reference counting is disabled so acquire and
 * release stay idempotent, matching the [NetworkSessionLock] contract.
 *
 * `WIFI_MODE_FULL_LOW_LATENCY` requires API 29 -- below that, `WifiService`
 * does not recognize the lock type at all (confirmed on a real Android 8.0
 * device, 2026-08-09: `dumpsys wifi`'s lock-tracking output has buckets only
 * for "full"/"full high perf"/"scan", nothing for "low latency"). Silently
 * acquiring a lock type the platform does not track back is not the same as
 * silently falling back to no lock at all -- so this falls back to
 * `WIFI_MODE_FULL_HIGH_PERF`, which has existed since early Android and
 * prevents the same Wi-Fi power-save sleep, just without the additional
 * radio-scheduling tuning `LOW_LATENCY` adds on top on 29+.
 */
class WifiLowLatencyNetworkLock(application: Application) : NetworkSessionLock {
    private val lock = (application.getSystemService(Context.WIFI_SERVICE) as WifiManager)
        .createWifiLock(lockMode(), "silent-disco:manual-listener")
        .apply { setReferenceCounted(false) }

    override fun acquire() {
        if (!lock.isHeld) lock.acquire()
    }

    override fun release() {
        if (lock.isHeld) lock.release()
    }

    private companion object {
        @Suppress("DEPRECATION") // deliberate pre-API-29 fallback; see the class doc comment
        fun lockMode(): Int = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            WifiManager.WIFI_MODE_FULL_LOW_LATENCY
        } else {
            WifiManager.WIFI_MODE_FULL_HIGH_PERF
        }
    }
}
