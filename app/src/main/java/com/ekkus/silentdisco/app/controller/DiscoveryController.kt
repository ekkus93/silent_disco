package com.ekkus.silentdisco.app.controller

import com.ekkus.silentdisco.app.toFfiSessionAdvertisement
import com.ekkus.silentdisco.core.rust.ListenerCoreController
import com.ekkus.silentdisco.core.transport.BleTransport
import com.ekkus.silentdisco.core.transport.MdnsSessionAdvertisement
import com.ekkus.silentdisco.core.transport.MdnsTransport
import com.ekkus.silentdisco.core.transport.SessionTransport
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch

/**
 * Session discovery for the listener role: the discovery radios (mDNS, BLE
 * scanning, Wi-Fi Direct peer discovery), the scan window, and reconciling
 * what they find into the listener core (God-file refactor spec §4).
 *
 * The radios are shared services (spec A.3 item 7): the host role also
 * advertises over BLE and Wi-Fi Direct, so failure routing and the Wi-Fi
 * Direct connection snapshot stay with the coordinator. The core decides
 * what a discovered session means; this only reports what the radios see.
 *
 * @param listenerCore the listener core, if one exists yet.
 */
internal class DiscoveryController(
    parentScope: CoroutineScope,
    private val ble: BleTransport,
    private val mdns: MdnsTransport,
    private val wifiDirect: SessionTransport,
    private val listenerCore: () -> ListenerCoreController?,
) : ViewModelController(parentScope) {
    private var scanWindow: Job? = null

    /** Desktop sessions resolved over mDNS, with their advertised endpoint versions. */
    val mdnsSessions: StateFlow<List<MdnsSessionAdvertisement>> get() = mdns.discoveredSessions

    /** Reconciles into the core whenever BLE or mDNS results change. */
    fun observe() {
        scope.launch { ble.discoveredSessions.collect { reconcile() } }
        scope.launch { mdns.discoveredSessions.collect { reconcile() } }
    }

    /**
     * Submits every current advertisement and expires the ones no radio sees
     * any more. Only while the core is discovering (Block 52.3): stopping
     * mDNS clears its list, so reconciling afterwards expired the session the
     * listener had just joined, and resubmitted results the core rejects once
     * discovery is inactive. Results already found when discovery starts are
     * submitted by the snapshot handler instead.
     */
    fun reconcile() {
        val controller = listenerCore() ?: return
        if (controller.snapshots.value?.discoveryActive != true) return
        val mdnsSessions = mdns.discoveredSessions.value
            .distinctBy { it.sessionId }
            .sortedBy { it.sessionName }
        val mdnsSessionIds = mdnsSessions.map { it.sessionId }.toSet()
        val bleSessions = ble.discoveredSessions.value
            .filter { it.id !in mdnsSessionIds }
            .distinctBy { it.id }
            .sortedBy { it.name }

        // Re-submit current advertisements, not only new IDs. The Rust actor
        // replaces an existing advertisement with the same session ID, which
        // is how an endpoint-less BLE discovery is upgraded when mDNS later
        // resolves the same desktop session.
        mdnsSessions.forEach { controller.submitSessionDiscovered(it.toFfiSessionAdvertisement()) }
        bleSessions.forEach { controller.submitSessionDiscovered(it.toFfiSessionAdvertisement()) }

        val desiredSessionIds = mdnsSessionIds + bleSessions.map { it.id }
        val known = controller.snapshots.value?.discoveredSessions.orEmpty()
            .map { it.sessionId }
            .toSet()
        (known - desiredSessionIds).forEach(controller::submitSessionExpired)
    }

    /**
     * Starts the radios this device may use. mDNS is a standard-IP
     * convenience layer that needs no nearby-device permission, so it starts
     * first: a desktop host stays discoverable even when the user declines
     * the optional radios. Returns null when at least one radio started,
     * otherwise why none did.
     */
    fun start(hasNearbyPermissions: Boolean): String? {
        val mdnsResult = mdns.startDiscovery()
        if (!hasNearbyPermissions) {
            return if (mdnsResult.started) {
                null
            } else {
                mdnsResult.message ?: "Missing nearby connectivity permissions and mDNS discovery is unavailable"
            }
        }
        val bleResult = ble.startScanning()
        if (bleResult.started) {
            wifiDirect.discoverPeers()
        }
        return if (mdnsResult.started || bleResult.started) {
            null
        } else {
            bleResult.message ?: mdnsResult.message ?: "No discovery backend could start"
        }
    }

    /** Stops every radio, attempting all of them; returns the first failure, others suppressed. */
    fun stop(): RuntimeException? {
        var firstFailure: RuntimeException? = null
        fun attempt(action: () -> Unit) {
            try {
                action()
            } catch (error: RuntimeException) {
                val first = firstFailure
                if (first == null) firstFailure = error else first.addSuppressed(error)
            }
        }
        attempt(mdns::stopDiscovery)
        attempt(ble::stopScanning)
        attempt(wifiDirect::cancelDiscovery)
        return firstFailure
    }

    /** Reconciles once more after [windowMs], replacing any running window. */
    fun startScanWindow(windowMs: Long) {
        scanWindow?.cancel()
        scanWindow = scope.launch {
            delay(windowMs)
            reconcile()
        }
    }

    fun cancelScanWindow() {
        scanWindow?.cancel()
        scanWindow = null
    }
}
