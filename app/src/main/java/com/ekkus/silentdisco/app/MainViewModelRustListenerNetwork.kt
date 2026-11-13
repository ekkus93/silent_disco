package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.transport.TransportSnapshot

/** Protocol version advertised by this app's BLE and desktop-mDNS discovery layers. */
internal const val LISTENER_DISCOVERY_PROTOCOL_VERSION: Int = 3

/** Called from the Wi-Fi Direct snapshot collector; see `ListenerSessionController`. */
internal fun MainViewModel.completeRustListenerNetworkEstablishment(snapshot: TransportSnapshot) =
    listenerSession.completeWifiDirectEstablishment(snapshot)
