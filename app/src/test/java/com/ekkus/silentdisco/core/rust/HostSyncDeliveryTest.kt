package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiHostTransportDelivery
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class HostSyncDeliveryTest {
    private fun delivery(intended: UInt, successful: UInt, failed: UInt) =
        FfiHostTransportDelivery(
            intendedPeers = intended,
            successfulPeers = successful,
            failedPeers = failed,
            bytesSent = if (successful > 0u) 64uL else 0uL,
        )

    @Test
    fun disconnectedNativeHandleIsNotACompletedClockResponse() {
        val error = runCatching { requireDeliveredHostSyncResponse(null) }.exceptionOrNull()
        assertThat(error).isInstanceOf(IllegalStateException::class.java)
        assertThat(error?.message).contains("disconnected before synchronization response delivery")
    }

    @Test
    fun zeroRecipientsAndFailedDatagramsAreNotClockResponses() {
        for (report in listOf(delivery(0u, 0u, 0u), delivery(1u, 0u, 1u))) {
            val error = runCatching { requireDeliveredHostSyncResponse(report) }.exceptionOrNull()
            assertThat(error).isInstanceOf(IllegalStateException::class.java)
            assertThat(error?.message).contains("did not reach any listener")
        }
    }

    @Test
    fun actualDatagramDeliveryPreservesNativeReport() {
        val report = delivery(2u, 1u, 1u)
        assertThat(requireDeliveredHostSyncResponse(report)).isSameInstanceAs(report)
    }
}
