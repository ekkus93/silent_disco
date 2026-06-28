package com.ekkus.silentdisco.core.transport

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class TransportOperationResultTest {

    @Test
    fun started_hasStartedTrue() {
        assertThat(TransportOperationResult.Started.started).isTrue()
        assertThat(TransportOperationResult.Started.message).isNull()
    }

    @Test
    fun failed_hasStartedFalse() {
        val result = TransportOperationResult.failed("something went wrong")
        assertThat(result.started).isFalse()
        assertThat(result.message).isEqualTo("something went wrong")
    }
}

class SendAllResultTest {

    @Test
    fun deliveredToAnyPeer_trueWhenSuccessCountPositive() {
        val result = SendAllResult(peerCount = 3, successCount = 2, failureCount = 1)
        assertThat(result.deliveredToAnyPeer).isTrue()
    }

    @Test
    fun deliveredToAnyPeer_falseWhenSuccessCountZero() {
        val result = SendAllResult(peerCount = 3, successCount = 0, failureCount = 3)
        assertThat(result.deliveredToAnyPeer).isFalse()
    }

    @Test
    fun allDelivered_trueWhenNoPeersFailedAndPeerCountPositive() {
        val result = SendAllResult(peerCount = 2, successCount = 2, failureCount = 0)
        assertThat(result.allDelivered).isTrue()
    }

    @Test
    fun allDelivered_falseWhenZeroPeers() {
        val result = SendAllResult(peerCount = 0, successCount = 0, failureCount = 0)
        assertThat(result.allDelivered).isFalse()
    }

    @Test
    fun allDelivered_falseWhenAnyFailure() {
        val result = SendAllResult(peerCount = 2, successCount = 1, failureCount = 1)
        assertThat(result.allDelivered).isFalse()
    }

    @Test
    fun successAndFailureCountsSumToPeerCount() {
        val result = SendAllResult(peerCount = 5, successCount = 3, failureCount = 2)
        assertThat(result.successCount + result.failureCount).isEqualTo(result.peerCount)
    }
}
