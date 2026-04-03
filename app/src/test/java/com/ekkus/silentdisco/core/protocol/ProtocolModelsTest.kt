package com.ekkus.silentdisco.core.protocol

import com.google.common.truth.Truth.assertThat
import kotlinx.serialization.json.Json
import org.junit.Test

class ProtocolModelsTest {
    private val json = Json {
        encodeDefaults = true
        ignoreUnknownKeys = true
        classDiscriminator = "type"
    }

    @Test
    fun `control message serializes and deserializes`() {
        val original: ControlMessage = ControlMessage.JoinRequest(
            version = 1,
            sessionId = SessionId("session-a"),
            device = DeviceIdentity("device-1", "Pixel"),
            inviteCode = "1234",
        )

        val serialized = json.encodeToString(ControlMessage.serializer(), original)
        val decoded = json.decodeFromString(ControlMessage.serializer(), serialized)

        assertThat(decoded).isEqualTo(original)
    }
}
