package com.ekkus.silentdisco.core.audio

import com.google.common.truth.Truth.assertThat
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import org.junit.Test

class PcmPacketizerTest {
    @Test
    fun `packetizer creates timestamped packets`() {
        val packetizer = PcmPacketizer(
            sessionId = SessionId("session"),
            streamId = StreamId("stream"),
        )
        val chunk = DecodedAudioChunk(
            pcm16Le = ByteArray(960 * 4 * 2),
            firstSampleIndex = 0,
            frameCount = 1_920,
        )

        val packets = packetizer.packetize(chunk, hostPresentationStartMs = 1_000)

        assertThat(packets).hasSize(2)
        assertThat(packets.first().hostPresentationTimeMs).isEqualTo(1_000)
        assertThat(packets.last().hostPresentationTimeMs).isEqualTo(1_020)
        val budget = packets.validatePacketBudget()
        assertThat(budget.valid).isTrue()
        assertThat(budget.maxPacketBytes).isGreaterThan(0)
    }
}
