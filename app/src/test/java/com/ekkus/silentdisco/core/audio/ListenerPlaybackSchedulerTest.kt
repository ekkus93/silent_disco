package com.ekkus.silentdisco.core.audio

import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.sync.HostTimeMapper
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class ListenerPlaybackSchedulerTest {
    @Test
    fun `scheduler tracks packet ordering and playback readiness`() {
        val scheduler = ListenerPlaybackScheduler(
            mapper = HostTimeMapper(offsetMs = 0.0),
            thresholds = PlaybackThresholds(startupBufferMs = 20),
            nowProvider = { 0L },
        )

        scheduler.submit(packet(sequence = 0, hostTimeMs = 20))
        scheduler.submit(packet(sequence = 2, hostTimeMs = 60))
        val telemetry = scheduler.submit(packet(sequence = 1, hostTimeMs = 40))

        assertThat(scheduler.canStart()).isTrue()
        assertThat(telemetry.packetLossCount).isEqualTo(0)
        assertThat(scheduler.poll(nowLocalTimeMs = 25)?.packet?.sequenceNumber).isEqualTo(0)
        assertThat(scheduler.poll(nowLocalTimeMs = 45)?.packet?.sequenceNumber).isEqualTo(1)
        assertThat(scheduler.poll(nowLocalTimeMs = 65)?.packet?.sequenceNumber).isEqualTo(2)
    }

    private fun packet(sequence: Long, hostTimeMs: Long) = AudioPacket(
        version = 1,
        sessionId = SessionId("session"),
        streamId = StreamId("stream"),
        sequenceNumber = sequence,
        codec = "pcm16le",
        sampleRate = 48_000,
        channelCount = 2,
        samplesPerPacket = 960,
        firstSampleIndex = sequence * 960,
        hostPresentationTimeMs = hostTimeMs,
        payload = ByteArray(64),
        checksum = null,
    )
}
