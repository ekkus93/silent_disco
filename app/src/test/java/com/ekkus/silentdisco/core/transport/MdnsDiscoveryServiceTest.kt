package com.ekkus.silentdisco.core.transport

import com.ekkus.silentdisco.core.model.ApprovalMode
import com.google.common.truth.Truth.assertThat
import java.nio.charset.StandardCharsets
import org.junit.Test

class MdnsDiscoveryServiceTest {
    private fun attrs(vararg entries: Pair<String, String>): Map<String, ByteArray> =
        entries.associate { (key, value) -> key to value.toByteArray(StandardCharsets.UTF_8) }

    @Test
    fun parserReconstructsTheFullDesktopAdvertisement() {
        val advertisement = parseMdnsAdvertisement(
            MdnsResolvedRecord(
                serviceName = "session-1",
                address = "192.168.1.50",
                servicePort = 41_100,
                attributes = attrs(
                    "sessionId" to "session-1",
                    "hostDeviceId" to "host-1",
                    "sessionName" to "Patio Mix",
                    "approvalMode" to "invite_code",
                    "protocolVersion" to "2",
                    "controlPort" to "41100",
                    "syncPort" to "41101",
                    "audioPort" to "41102",
                    "inviteCodeRequired" to "true",
                ),
            ),
        )

        assertThat(advertisement).isEqualTo(
            MdnsSessionAdvertisement(
                sessionId = "session-1",
                hostDeviceId = "host-1",
                sessionName = "Patio Mix",
                approvalMode = ApprovalMode.INVITE_CODE,
                protocolVersion = 2,
                address = "192.168.1.50",
                controlPort = 41_100,
                syncPort = 41_101,
                audioPort = 41_102,
            ),
        )
    }

    @Test
    fun parserRejectsConflictingControlPortsRatherThanGuessing() {
        val advertisement = parseMdnsAdvertisement(
            MdnsResolvedRecord(
                serviceName = "session-1",
                address = "192.168.1.50",
                servicePort = 41_100,
                attributes = attrs(
                    "sessionId" to "session-1",
                    "hostDeviceId" to "host-1",
                    "sessionName" to "Patio Mix",
                    "approvalMode" to "manual",
                    "protocolVersion" to "2",
                    "controlPort" to "49999",
                    "syncPort" to "41101",
                    "audioPort" to "41102",
                ),
            ),
        )

        assertThat(advertisement).isNull()
    }

    @Test
    fun parserRejectsUnknownApprovalModes() {
        val advertisement = parseMdnsAdvertisement(
            MdnsResolvedRecord(
                serviceName = "session-1",
                address = "192.168.1.50",
                servicePort = 41_100,
                attributes = attrs(
                    "sessionId" to "session-1",
                    "hostDeviceId" to "host-1",
                    "sessionName" to "Patio Mix",
                    "approvalMode" to "auto_approve_everyone",
                    "protocolVersion" to "2",
                    "syncPort" to "41101",
                    "audioPort" to "41102",
                ),
            ),
        )

        assertThat(advertisement).isNull()
    }

    @Test
    fun parserRejectsUnsupportedProtocolVersions() {
        val advertisement = parseMdnsAdvertisement(
            MdnsResolvedRecord(
                serviceName = "session-3",
                address = "192.168.1.50",
                servicePort = 41_100,
                attributes = attrs(
                    "sessionId" to "session-3",
                    "hostDeviceId" to "host-3",
                    "sessionName" to "Future Mix",
                    "approvalMode" to "manual",
                    "protocolVersion" to "3",
                    "syncPort" to "41101",
                    "audioPort" to "41102",
                ),
            ),
        )

        assertThat(advertisement).isNull()
    }

    @Test
    fun parserStripsIpv6ScopeIdsBeforePassingTheAddressToRust() {
        val advertisement = parseMdnsAdvertisement(
            MdnsResolvedRecord(
                serviceName = "session-v6",
                address = "fe80::1234%wlan0",
                servicePort = 41_100,
                attributes = attrs(
                    "sessionId" to "session-v6",
                    "hostDeviceId" to "host-v6",
                    "sessionName" to "IPv6 Mix",
                    "approvalMode" to "manual",
                    "protocolVersion" to "2",
                    "syncPort" to "41101",
                    "audioPort" to "41102",
                ),
            ),
        )

        assertThat(advertisement?.address).isEqualTo("fe80::1234")
    }
}
