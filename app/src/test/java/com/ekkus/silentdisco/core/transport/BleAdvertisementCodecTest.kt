package com.ekkus.silentdisco.core.transport

import com.google.common.truth.Truth.assertThat
import org.junit.Test

// Legacy BLE advertising's hard limit: each of the two packets (primary and
// scan response) is capped at 31 bytes of raw advertising data. This is the
// exact real-world cap that a previous, larger payload violated (observed on
// device as ADVERTISE_FAILED_DATA_TOO_LARGE, error code 1), so it is
// asserted directly rather than just trusted.
private const val LEGACY_SCAN_RESPONSE_LIMIT_BYTES = 31

// The "Service Data" advertising-data structure's fixed overhead for a
// 16-bit UUID key: 1-byte length + 1-byte type + 2-byte UUID.
private const val SERVICE_DATA_16_BIT_UUID_OVERHEAD_BYTES = 4

class BleAdvertisementCodecTest {
    @Test
    fun `codec round trips session metadata needed for discovery`() {
        val advertisement = BleAdvertisement(
            sessionId = "550e8400-e29b-41d4-a716-446655440000",
            sessionName = "Patio Mix",
            hostName = "unused-app-level-host-id",
            approvalRequired = true,
            inviteCodeRequired = true,
        )

        val decoded = BleAdvertisementCodec.decode(
            payload = BleAdvertisementCodec.encode(advertisement, hostDeviceName = "Pixel Host"),
        )

        assertThat(decoded?.sessionName).isEqualTo("Patio ")
        assertThat(decoded?.hostName).isEqualTo("Pixel Host")
        assertThat(decoded?.approvalRequired).isTrue()
        assertThat(decoded?.inviteCodeRequired).isTrue()
    }

    @Test
    fun `decoded session id is a stable, non-empty opaque key`() {
        val advertisement = BleAdvertisement(
            sessionId = "550e8400-e29b-41d4-a716-446655440000",
            sessionName = "Session",
            hostName = "",
            approvalRequired = false,
            inviteCodeRequired = false,
        )

        val first = BleAdvertisementCodec.decode(BleAdvertisementCodec.encode(advertisement, ""))
        val second = BleAdvertisementCodec.decode(BleAdvertisementCodec.encode(advertisement, ""))

        assertThat(first?.sessionId).isNotEmpty()
        assertThat(first?.sessionId).isEqualTo(second?.sessionId)
    }

    @Test
    fun `encoded payload always fits the legacy scan-response packet even with long names`() {
        val advertisement = BleAdvertisement(
            sessionId = "550e8400-e29b-41d4-a716-446655440000",
            sessionName = "A Very Long Session Name That Will Not Fit",
            hostName = "unused",
            approvalRequired = true,
            inviteCodeRequired = true,
        )
        val longDeviceName = "Phillip's Samsung Galaxy A54 5G Ultra Max"

        val payload = BleAdvertisementCodec.encode(advertisement, hostDeviceName = longDeviceName)
        val totalPacketBytes = SERVICE_DATA_16_BIT_UUID_OVERHEAD_BYTES + payload.size

        assertThat(totalPacketBytes).isAtMost(LEGACY_SCAN_RESPONSE_LIMIT_BYTES)
        assertThat(BleAdvertisementCodec.decode(payload)).isNotNull()
    }

    @Test
    fun `decode rejects a payload from a different protocol version`() {
        val payload = BleAdvertisementCodec.encode(
            BleAdvertisement(
                sessionId = "550e8400-e29b-41d4-a716-446655440000",
                sessionName = "Session",
                hostName = "",
                approvalRequired = false,
                inviteCodeRequired = false,
            ),
            hostDeviceName = "",
        )
        payload[0] = 1 // the previous protocol version

        assertThat(BleAdvertisementCodec.decode(payload)).isNull()
    }

    @Test
    fun `decode rejects a payload shorter than the minimum size`() {
        assertThat(BleAdvertisementCodec.decode(ByteArray(3))).isNull()
    }

    @Test
    fun `decode rejects a payload whose declared host name length overruns the buffer`() {
        // version=2, flags=0, sessionId=6 zero bytes, hostNameLength=100 (far
        // larger than the single trailing byte actually available).
        val malformed = byteArrayOf(2, 0, 0, 0, 0, 0, 0, 0, 100, 0)
        assertThat(BleAdvertisementCodec.decode(malformed)).isNull()
    }
}
