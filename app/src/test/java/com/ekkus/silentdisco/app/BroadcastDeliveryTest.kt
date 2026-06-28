package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.transport.BroadcastDeliverySeverity
import com.ekkus.silentdisco.core.transport.SendAllResult
import com.ekkus.silentdisco.core.transport.classifyBroadcastDelivery
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class BroadcastDeliveryTest {

    @Test
    fun classifyBroadcastDelivery_zeroPeersIsNotSuccess() {
        val report = classifyBroadcastDelivery(
            "broadcast pause",
            SendAllResult(peerCount = 0, successCount = 0, failureCount = 0),
        )
        assertEquals(BroadcastDeliverySeverity.ZERO_PEERS, report.severity)
        assertTrue(report.message?.contains("no connected listeners") == true)
    }

    @Test
    fun classifyBroadcastDelivery_partialFailureIsWarning() {
        val report = classifyBroadcastDelivery(
            "broadcast pause",
            SendAllResult(peerCount = 3, successCount = 2, failureCount = 1),
        )
        assertEquals(BroadcastDeliverySeverity.PARTIAL_FAILURE, report.severity)
        assertTrue(report.message?.contains("2/3") == true)
    }

    @Test
    fun classifyBroadcastDelivery_allDeliveredIsOk() {
        val report = classifyBroadcastDelivery(
            "broadcast pause",
            SendAllResult(peerCount = 2, successCount = 2, failureCount = 0),
        )
        assertEquals(BroadcastDeliverySeverity.OK, report.severity)
        assertNull(report.message)
    }

    @Test
    fun classifyBroadcastDelivery_zeroPeersMessageIncludesAction() {
        val report = classifyBroadcastDelivery(
            "send join approval",
            SendAllResult(peerCount = 0, successCount = 0, failureCount = 0),
        )
        assertTrue(report.message?.contains("send join approval") == true)
    }

    @Test
    fun classifyBroadcastDelivery_partialMessageIncludesCountsAndAction() {
        val report = classifyBroadcastDelivery(
            "broadcast sync response",
            SendAllResult(peerCount = 4, successCount = 1, failureCount = 3),
        )
        assertTrue(report.message?.contains("broadcast sync response") == true)
        assertTrue(report.message?.contains("1/4") == true)
        assertTrue(report.message?.contains("3 failed") == true)
    }

    @Test
    fun classifyBroadcastDelivery_singlePeerAllDeliveredIsOk() {
        val report = classifyBroadcastDelivery(
            "send join rejection",
            SendAllResult(peerCount = 1, successCount = 1, failureCount = 0),
        )
        assertEquals(BroadcastDeliverySeverity.OK, report.severity)
    }
}
