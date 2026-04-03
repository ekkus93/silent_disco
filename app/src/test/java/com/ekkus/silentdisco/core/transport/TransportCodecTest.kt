package com.ekkus.silentdisco.core.transport

import com.google.common.truth.Truth.assertThat
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.ControlMessage
import com.ekkus.silentdisco.core.protocol.DeviceIdentity
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.protocol.SyncRequestPacket
import org.junit.Test

class TransportCodecTest {
    @Test
    fun `control codec round trips join request`() {
        val message: ControlMessage = ControlMessage.JoinRequest(
            version = 1,
            sessionId = SessionId("session-1"),
            device = DeviceIdentity(deviceId = "listener-1", displayName = "Pixel"),
            inviteCode = "2468",
        )

        val encoded = ControlMessageCodec.encode(message)
        val decoded = ControlMessageCodec.decode(encoded)

        assertThat(decoded).isEqualTo(message)
    }

    @Test
    fun `sync request codec round trips probe`() {
        val packet = SyncRequestPacket(
            version = 1,
            sessionId = SessionId("session-1"),
            correlationId = 99L,
            t1ListenerSendElapsedMs = 1_234L,
        )

        val encoded = SyncRequestPacketCodec.encode(packet)
        val decoded = SyncRequestPacketCodec.decode(encoded)

        assertThat(decoded).isEqualTo(packet)
    }

    @Test
    fun `audio codec round trips packet fields and payload`() {
        val packet = AudioPacket(
            version = 1,
            sessionId = SessionId("session-1"),
            streamId = StreamId("stream-1"),
            sequenceNumber = 7L,
            codec = "pcm16le",
            sampleRate = 48_000,
            channelCount = 2,
            samplesPerPacket = 960,
            firstSampleIndex = 6_720L,
            hostPresentationTimeMs = 5_000L,
            payload = byteArrayOf(1, 2, 3, 4, 5, 6),
            checksum = 12345,
        )

        val encoded = AudioPacketCodec.encode(packet)
        val decoded = AudioPacketCodec.decode(encoded)

        assertThat(decoded.version).isEqualTo(packet.version)
        assertThat(decoded.sessionId).isEqualTo(packet.sessionId)
        assertThat(decoded.streamId).isEqualTo(packet.streamId)
        assertThat(decoded.sequenceNumber).isEqualTo(packet.sequenceNumber)
        assertThat(decoded.codec).isEqualTo(packet.codec)
        assertThat(decoded.sampleRate).isEqualTo(packet.sampleRate)
        assertThat(decoded.channelCount).isEqualTo(packet.channelCount)
        assertThat(decoded.samplesPerPacket).isEqualTo(packet.samplesPerPacket)
        assertThat(decoded.firstSampleIndex).isEqualTo(packet.firstSampleIndex)
        assertThat(decoded.hostPresentationTimeMs).isEqualTo(packet.hostPresentationTimeMs)
        assertThat(decoded.checksum).isEqualTo(packet.checksum)
        assertThat(decoded.payload.toList())
            .containsExactly(1.toByte(), 2.toByte(), 3.toByte(), 4.toByte(), 5.toByte(), 6.toByte())
            .inOrder()
    }
}
