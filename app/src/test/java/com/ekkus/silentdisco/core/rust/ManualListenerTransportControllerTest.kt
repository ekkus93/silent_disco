package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.model.ManualConnectUiState
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportException
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class ManualListenerTransportControllerTest {
    @Test
    fun audioReceivedMapsEveryFieldIntoTheSharedPacketModel() {
        val event = FfiListenerTransportEvent.AudioReceived(
            streamId = "stream-1",
            sequence = 7uL,
            sampleRate = 48_000u,
            channels = 2u,
            samplesPerPacket = 960u,
            firstSampleIndex = 6_720uL,
            hostPresentationTimeMs = 123_456uL,
            payload = byteArrayOf(1, 2, 3, 4),
        )

        val packet = mapAudioReceivedToPacket(event, SessionId("session-1"), protocolVersion = 2)

        assertThat(packet.version).isEqualTo(2)
        assertThat(packet.sessionId).isEqualTo(SessionId("session-1"))
        assertThat(packet.streamId).isEqualTo(StreamId("stream-1"))
        assertThat(packet.sequenceNumber).isEqualTo(7L)
        assertThat(packet.codec).isEqualTo("pcm16le")
        assertThat(packet.sampleRate).isEqualTo(48_000)
        assertThat(packet.channelCount).isEqualTo(2)
        assertThat(packet.samplesPerPacket).isEqualTo(960)
        assertThat(packet.firstSampleIndex).isEqualTo(6_720L)
        assertThat(packet.hostPresentationTimeMs).isEqualTo(123_456L)
        assertThat(packet.payload).isEqualTo(byteArrayOf(1, 2, 3, 4))
    }

    @Test
    fun everyPostConnectionExceptionMapsToDisconnectedNotFailed() {
        val exceptions = listOf(
            FfiListenerTransportException.InvalidEndpoint("invalid"),
            FfiListenerTransportException.UnsupportedProtocolVersion("version"),
            FfiListenerTransportException.Unauthorized("unauthorized"),
            FfiListenerTransportException.Timeout("timeout"),
            FfiListenerTransportException.ConnectionFailed("connection failed"),
            FfiListenerTransportException.Io("io"),
            FfiListenerTransportException.Closed("transport event channel is closed"),
        )

        for (exception in exceptions) {
            val state = mapPostConnectionFailure(exception)
            assertThat(state).isEqualTo(ManualConnectUiState.Disconnected(exception.message))
        }
    }

    @Test
    fun closedExceptionIsNotRenderedAsAConnectionFailure() {
        val state = mapPostConnectionFailure(
            FfiListenerTransportException.Closed("transport event channel is closed"),
        )

        assertThat(state).isNotInstanceOf(ManualConnectUiState.Failed::class.java)
    }
}
