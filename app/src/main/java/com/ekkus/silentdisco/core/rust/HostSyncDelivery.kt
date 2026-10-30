package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiHostTransportDelivery

/** A clock response is authoritative: zero successful datagrams is not delivery. */
internal fun requireDeliveredHostSyncResponse(
    delivery: FfiHostTransportDelivery?,
): FfiHostTransportDelivery {
    val report = checkNotNull(delivery) {
        "Host transport disconnected before synchronization response delivery"
    }
    check(report.successfulPeers > 0u) {
        "Host synchronization response did not reach any listener"
    }
    return report
}
