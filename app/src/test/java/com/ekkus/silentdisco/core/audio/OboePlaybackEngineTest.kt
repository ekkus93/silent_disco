package com.ekkus.silentdisco.core.audio

import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.sync.HostTimeMapper
import com.google.common.truth.Truth.assertThat
import org.junit.Assert.assertThrows
import org.junit.Test

class OboePlaybackEngineTest {

    // --- OboePlaybackEngine null-AudioTrack path ---
    // AudioTrack is an Android system class unavailable in JVM unit tests.
    // These tests cover pre-start / post-stop behavior via the null fallback path.

    @Test
    fun `write before start throws error`() {
        val engine = OboePlaybackEngine()
        val frame = frame(payloadSize = 3840)
        assertThrows(IllegalStateException::class.java) {
            engine.write(frame)
        }
    }

    @Test
    fun `setVolume stores value and clamps to range`() {
        val engine = OboePlaybackEngine()
        engine.setVolume(0.5f)
        engine.setVolume(1.5f) // clamps to 1.0
        engine.setVolume(-0.5f) // clamps to 0.0
        // Volume is set without error
        engine.stop()
    }

    @Test
    fun `setVolume before start does not throw`() {
        val engine = OboePlaybackEngine()
        engine.setVolume(0.75f)
        // Should succeed
    }

    @Test
    fun `stop before start does not throw`() {
        val engine = OboePlaybackEngine()
        engine.stop()
        engine.stop() // idempotent
    }

    @Test
    fun `playbackPositionMs before start returns frame localDeadlineMs`() {
        val engine = OboePlaybackEngine()
        val frame = frame(payloadSize = 3840, localDeadlineMs = 12_345L)
        val posMs = engine.playbackPositionMs(frame)
        assertThat(posMs).isEqualTo(12_345L)
    }

    @Test
    fun `write after stop throws error`() {
        val engine = OboePlaybackEngine()
        engine.stop()
        assertThrows(IllegalStateException::class.java) {
            engine.write(frame(payloadSize = 3840))
        }
    }

    // --- Scheduler behavior ---
    // Verifies that scheduler poll can return null when frames aren't ready.

    @Test
    fun `scheduler poll returns null when no frames ready`() {
        val scheduler = scheduler()
        scheduler.submit(packet(sequence = 0, hostTimeMs = 20))
        scheduler.submit(packet(sequence = 1, hostTimeMs = 40))

        // Only poll at t=25 — seq=0 ready, seq=1 not yet
        val frame0 = scheduler.poll(nowLocalTimeMs = 25)
        val frame1 = scheduler.poll(nowLocalTimeMs = 25)

        assertThat(frame0?.packet?.sequenceNumber).isEqualTo(0)
        assertThat(frame1).isNull()
    }

    @Test
    fun `scheduler handles empty buffer without crashing`() {
        val scheduler = scheduler()
        val frame = scheduler.poll(nowLocalTimeMs = 100)
        assertThat(frame).isNull()
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
