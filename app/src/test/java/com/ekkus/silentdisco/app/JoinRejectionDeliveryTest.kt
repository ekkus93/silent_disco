package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.transport.BroadcastDeliverySeverity
import com.ekkus.silentdisco.core.transport.SendAllResult
import com.ekkus.silentdisco.core.transport.classifyBroadcastDelivery
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class JoinRejectionDeliveryTest {

    // rejectJoinRequest uses requireAnyPeer = true: zero peers means not delivered.

    @Test
    fun zeroPeerRejection_isNotDelivered() {
        val result = SendAllResult(peerCount = 0, successCount = 0, failureCount = 0)
        val report = classifyBroadcastDelivery("send join rejection", result)
        assertEquals(BroadcastDeliverySeverity.ZERO_PEERS, report.severity)
        // requireAnyPeer = true means ZERO_PEERS severity should not count as success
        assertFalse("Zero peers with requireAnyPeer=true must not be treated as success", result.deliveredToAnyPeer)
    }

    @Test
    fun fullDeliveryRejection_isSuccess() {
        val result = SendAllResult(peerCount = 1, successCount = 1, failureCount = 0)
        val report = classifyBroadcastDelivery("send join rejection", result)
        assertEquals(BroadcastDeliverySeverity.OK, report.severity)
        assertTrue(result.deliveredToAnyPeer)
        assertTrue(result.allDelivered)
    }

    @Test
    fun partialDeliveryRejection_isPartialFailure() {
        val result = SendAllResult(peerCount = 2, successCount = 1, failureCount = 1)
        val report = classifyBroadcastDelivery("send join rejection", result)
        assertEquals(BroadcastDeliverySeverity.PARTIAL_FAILURE, report.severity)
        assertTrue(result.deliveredToAnyPeer)
        assertFalse(result.allDelivered)
    }

    @Test
    fun zeroPeerRejection_deliveryFailureIsVisible() {
        val result = SendAllResult(peerCount = 0, successCount = 0, failureCount = 0)
        val report = classifyBroadcastDelivery("send join rejection", result)
        // Delivery failure must surface — not silently succeed
        assertTrue(
            "Zero peer rejection must produce a non-OK delivery report",
            report.severity != BroadcastDeliverySeverity.OK,
        )
    }
}
