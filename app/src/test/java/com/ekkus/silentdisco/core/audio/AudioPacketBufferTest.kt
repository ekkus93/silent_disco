package com.ekkus.silentdisco.core.audio

import com.google.common.truth.Truth.assertThat
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import org.junit.Test

class AudioPacketBufferTest {
    @Test
    fun `buffer orders packets by sequence and reports readiness`() {
        val buffer = AudioPacketBuffer(startupTargetMs = 40)
        buffer.insert(packet(sequence = 2, localTime = 60))
        buffer.insert(packet(sequence = 0, localTime = 20))
        buffer.insert(packet(sequence = 1, localTime = 40))

        assertThat(buffer.isReady()).isTrue()
        assertThat(buffer.popReady(nowLocalTimeMs = 21)?.packet?.sequenceNumber).isEqualTo(0)
        assertThat(buffer.popReady(nowLocalTimeMs = 41)?.packet?.sequenceNumber).isEqualTo(1)
        assertThat(buffer.popReady(nowLocalTimeMs = 61)?.packet?.sequenceNumber).isEqualTo(2)
    }

    private fun packet(sequence: Long, localTime: Long) = BufferedAudioPacket(
        packet = AudioPacket(
            version = 1,
            sessionId = SessionId("session"),
            streamId = StreamId("stream"),
            sequenceNumber = sequence,
            codec = "pcm16le",
            sampleRate = 48_000,
            channelCount = 2,
            samplesPerPacket = 960,
            firstSampleIndex = sequence * 960,
            hostPresentationTimeMs = localTime,
            payload = ByteArray(32),
            checksum = null,
        ),
        scheduledLocalTimeMs = localTime,
    )
}
