package com.ekkus.silentdisco.core.transport

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class BleAdvertisementCodecTest {
    @Test
    fun `codec round trips session metadata needed for discovery`() {
        val advertisement = BleAdvertisement(
            sessionId = "550e8400-e29b-41d4-a716-446655440000",
            sessionName = "Patio Mix",
            hostName = "Pixel Host",
            approvalRequired = true,
            inviteCodeRequired = true,
        )

        val decoded = BleAdvertisementCodec.decode(
            payload = BleAdvertisementCodec.encode(advertisement),
            fallbackHostName = advertisement.hostName,
        )

        assertThat(decoded?.sessionId).isEqualTo(advertisement.sessionId)
        assertThat(decoded?.sessionName).isEqualTo("Patio Mi")
        assertThat(decoded?.hostName).isEqualTo(advertisement.hostName)
        assertThat(decoded?.approvalRequired).isTrue()
        assertThat(decoded?.inviteCodeRequired).isTrue()
    }
}
