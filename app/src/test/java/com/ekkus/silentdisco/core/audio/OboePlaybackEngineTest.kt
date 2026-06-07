package com.ekkus.silentdisco.core.audio

import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.sync.HostTimeMapper
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class OboePlaybackEngineTest {

    // --- OboePlaybackEngine null-AudioTrack path ---
    // AudioTrack is an Android system class unavailable in JVM unit tests.
    // These tests cover pre-start / post-stop behavior via the null fallback path.

    @Test
    fun `write before start returns payload size as sentinel`() {
        val engine = OboePlaybackEngine()
        val frame = frame(payloadSize = 3840)
        val written = engine.write(frame)
        assertThat(written).isEqualTo(3840L)
    }

    @Test
    fun `write with empty payload returns zero and does not increment write count`() {
        val engine = OboePlaybackEngine()
        val frame = frame(payloadSize = 0)
        val written = engine.write(frame)
        assertThat(written).isEqualTo(0L)
        assertThat(engine.statusSummary()).startsWith("writes=0")
    }

    @Test
    fun `write with non-empty payload increments write count`() {
        val engine = OboePlaybackEngine()
        engine.write(frame(payloadSize = 3840))
        engine.write(frame(payloadSize = 3840))
        assertThat(engine.statusSummary()).startsWith("writes=2")
    }

    @Test
    fun `stop before start does not throw`() {
        val engine = OboePlaybackEngine()
        engine.stop()
        engine.stop() // idempotent
    }

    @Test
    fun `stop is idempotent after write`() {
        val engine = OboePlaybackEngine()
        engine.write(frame(payloadSize = 3840))
        engine.stop()
        engine.stop() // should not crash
    }

    @Test
    fun `playbackPositionMs before start returns frame localDeadlineMs`() {
        val engine = OboePlaybackEngine()
        val frame = frame(payloadSize = 3840, localDeadlineMs = 12_345L)
        val posMs = engine.playbackPositionMs(frame)
        assertThat(posMs).isEqualTo(12_345L)
    }

    @Test
    fun `write after stop falls back to payload size`() {
        val engine = OboePlaybackEngine()
        engine.write(frame(payloadSize = 3840))
        engine.stop()
        val written = engine.write(frame(payloadSize = 3840))
        assertThat(written).isEqualTo(3840L)
    }

    // --- Write-loop integration with ListenerPlaybackScheduler ---
    // Simulates the poll-then-write loop in MainViewModel and verifies that
    // frames are consumed in deadline order and null returns are handled cleanly.

    @Test
    fun `write loop processes frames in deadline order`() {
        val scheduler = scheduler()
        val engine = OboePlaybackEngine()

        scheduler.submit(packet(sequence = 0, hostTimeMs = 20))
        scheduler.submit(packet(sequence = 1, hostTimeMs = 40))
        scheduler.submit(packet(sequence = 2, hostTimeMs = 60))

        val writtenSequences = mutableListOf<Long>()
        for (nowMs in listOf(25L, 45L, 65L)) {
            val frame = scheduler.poll(nowLocalTimeMs = nowMs)
            if (frame != null) {
                engine.write(frame)
                writtenSequences += frame.packet.sequenceNumber
            }
        }

        assertThat(writtenSequences).containsExactly(0L, 1L, 2L).inOrder()
        assertThat(engine.statusSummary()).startsWith("writes=3")
    }

    @Test
    fun `write loop skips null frames without crashing`() {
        val scheduler = scheduler()
        val engine = OboePlaybackEngine()

        scheduler.submit(packet(sequence = 0, hostTimeMs = 20))
        scheduler.submit(packet(sequence = 1, hostTimeMs = 40))

        // Only poll at t=25 — seq=1 is not yet ready, poll returns null
        val frame0 = scheduler.poll(nowLocalTimeMs = 25)
        val frame1 = scheduler.poll(nowLocalTimeMs = 25)

        if (frame0 != null) engine.write(frame0)
        if (frame1 != null) engine.write(frame1) // should be null; write skipped

        assertThat(frame0?.packet?.sequenceNumber).isEqualTo(0)
        assertThat(frame1).isNull()
        assertThat(engine.statusSummary()).startsWith("writes=1")
    }

    @Test
    fun `write loop handles empty scheduler without crashing`() {
        val scheduler = scheduler()
        val engine = OboePlaybackEngine()

        val frame = scheduler.poll(nowLocalTimeMs = 100)
        assertThat(frame).isNull()
        // No write called — count stays zero
        assertThat(engine.statusSummary()).startsWith("writes=0")
    }

    // --- Helpers ---

    private fun frame(payloadSize: Int, localDeadlineMs: Long = 0L): PlaybackFrame = PlaybackFrame(
        packet = packet(sequence = 0, hostTimeMs = localDeadlineMs, payloadSize = payloadSize),
        localDeadlineMs = localDeadlineMs,
    )

    private fun packet(
        sequence: Long,
        hostTimeMs: Long,
        payloadSize: Int = 3840,
    ) = AudioPacket(
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
        payload = ByteArray(payloadSize),
        checksum = null,
    )

    private fun scheduler() = ListenerPlaybackScheduler(
        mapper = HostTimeMapper(offsetMs = 0.0),
        thresholds = PlaybackThresholds(startupBufferMs = 20),
        expectedSessionId = SessionId("session"),
        expectedStreamId = StreamId("stream"),
        nowProvider = { 0L },
    )
}
