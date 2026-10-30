package com.ekkus.silentdisco.app

import com.google.common.truth.Truth.assertThat
import java.util.UUID
import org.junit.Test

class HostJoinRequestIdsTest {
    @Test
    fun repeatedJoinAttemptsFromTheSameClaimedDeviceHaveDistinctRequestIds() {
        val claimedDeviceId = "listener-1"
        val first = newHostJoinRequestId()
        val reconnect = newHostJoinRequestId()

        assertThat(first).isNotEqualTo(claimedDeviceId)
        assertThat(reconnect).isNotEqualTo(claimedDeviceId)
        assertThat(first).isNotEqualTo(reconnect)
        assertThat(UUID.fromString(first).toString()).isEqualTo(first)
        assertThat(UUID.fromString(reconnect).toString()).isEqualTo(reconnect)
    }

    @Test
    fun rapidReconnectAttemptsCannotReuseAStaleApprovalRequestId() {
        val attempts = (1..2_048).map { newHostJoinRequestId() }
        assertThat(attempts.toSet()).hasSize(attempts.size)
    }
}
