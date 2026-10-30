package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.uniffi.FfiDeliveryReport
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportDelivery

/**
 * Convert a native transport delivery into Rust actor accounting.
 * A targeted admission/control effect has one intended peer even when the
 * host handle was retired before dispatch; a missing native result is a
 * failure, not a successful zero-recipient no-op. Real reports remain intact.
 */
internal fun FfiHostTransportDelivery?.toFfiDeliveryReport(): FfiDeliveryReport = FfiDeliveryReport(
    intendedPeers = this?.intendedPeers ?: 1u,
    successfulPeers = this?.successfulPeers ?: 0u,
    failedPeers = this?.failedPeers ?: 1u,
)
