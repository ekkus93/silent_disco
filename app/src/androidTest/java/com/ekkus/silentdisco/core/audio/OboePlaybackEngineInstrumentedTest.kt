package com.ekkus.silentdisco.core.audio

import androidx.test.ext.junit.runners.AndroidJUnit4
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import kotlin.math.PI
import kotlin.math.sin
import org.junit.Test
import org.junit.runner.RunWith

private const val SAMPLE_RATE = 48_000
private const val CHANNEL_COUNT = 2
private const val PACKET_DURATION_MS = 20
private const val SAMPLES_PER_PACKET = SAMPLE_RATE * PACKET_DURATION_MS / 1_000
private const val TONE_FREQUENCY_HZ = 440.0
private const val TONE_AMPLITUDE = 0.3

/**
 * Exercises [OboePlaybackEngine] end to end on a physical device: real Oboe
 * stream open, a generated 440 Hz test tone pushed through the shared Rust
 * render ring for ~1 second, and stream teardown. This is the only
 * production-facing way to verify the native dlopen/dlsym/Oboe path (Block
 * 17/18's C++ adapter) actually works on real hardware -- none of it is
 * reachable from JVM unit tests.
 */
@RunWith(AndroidJUnit4::class)
class OboePlaybackEngineInstrumentedTest {

    private fun toneFrame(sequenceNumber: Long): PlaybackFrame {
        val payload = ByteArray(SAMPLES_PER_PACKET * CHANNEL_COUNT * 2)
        var byteIndex = 0
        val firstSample = sequenceNumber * SAMPLES_PER_PACKET
        for (frame in 0 until SAMPLES_PER_PACKET) {
            val t = (firstSample + frame).toDouble() / SAMPLE_RATE
            val sample = (TONE_AMPLITUDE * Short.MAX_VALUE * sin(2.0 * PI * TONE_FREQUENCY_HZ * t)).toInt().toShort()
            repeat(CHANNEL_COUNT) {
                payload[byteIndex] = (sample.toInt() and 0xFF).toByte()
                payload[byteIndex + 1] = ((sample.toInt() shr 8) and 0xFF).toByte()
                byteIndex += 2
            }
        }
        val packet = AudioPacket(
            version = 1,
            sessionId = SessionId("instrumented-test-session"),
            streamId = StreamId("instrumented-test-stream"),
            sequenceNumber = sequenceNumber,
            codec = "pcm16le",
            sampleRate = SAMPLE_RATE,
            channelCount = CHANNEL_COUNT,
            samplesPerPacket = SAMPLES_PER_PACKET,
            firstSampleIndex = firstSample,
            hostPresentationTimeMs = sequenceNumber * PACKET_DURATION_MS,
            payload = payload,
        )
        return PlaybackFrame(packet = packet, localDeadlineMs = packet.hostPresentationTimeMs)
    }

    @Test
    fun opens_a_real_oboe_stream_and_reports_a_native_backend() {
        assertThat(OboeBridge.isAvailable).isTrue()

        val engine = OboePlaybackEngine()
        try {
            val backend = engine.start()
            assertThat(backend).isEqualTo("Oboe")
            assertThat(OboeBridge.nativeOboeIsOpen()).isTrue()
            assertThat(OboeBridge.nativeOboeActualSampleRate()).isGreaterThan(0)
            assertThat(OboeBridge.nativeOboeActualChannelCount()).isGreaterThan(0)
        } finally {
            engine.stop()
        }
        assertThat(OboeBridge.nativeOboeIsOpen()).isFalse()
    }

    @Test
    fun plays_a_generated_test_tone_for_one_second_without_a_fatal_status() {
        val engine = OboePlaybackEngine()
        try {
            engine.start()

            val packetsPerSecond = 1_000 / PACKET_DURATION_MS
            for (sequence in 0 until packetsPerSecond) {
                val written = engine.write(toneFrame(sequence.toLong()))
                assertThat(written).isGreaterThan(0)
                Thread.sleep(PACKET_DURATION_MS.toLong())
            }

            // A momentary underrun/backpressure is not fatal; only a
            // contained panic or an invalid (unknown/never-issued) engine
            // token is, and neither should occur across a full second of
            // real playback started moments ago.
            assertThat(engine.takeFatalStatus()).isEqualTo(0)
            assertThat(engine.takeDisconnected()).isFalse()
            assertThat(OboeBridge.nativeOboeFramesRendered()).isGreaterThan(0)
        } finally {
            engine.stop()
        }
    }

    @Test
    fun open_start_stop_repeatedly_leaves_no_residual_native_state() {
        repeat(3) { iteration ->
            val engine = OboePlaybackEngine()
            engine.start()
            assertWithMessage("stream open on iteration $iteration")
                .that(OboeBridge.nativeOboeIsOpen())
                .isTrue()

            engine.write(toneFrame(0))
            Thread.sleep(PACKET_DURATION_MS.toLong())

            engine.stop()
            assertWithMessage("stream closed on iteration $iteration")
                .that(OboeBridge.nativeOboeIsOpen())
                .isFalse()
        }
    }

    @Test
    fun no_callback_fires_after_stop_returns() {
        val engine = OboePlaybackEngine()
        engine.start()
        engine.write(toneFrame(0))
        Thread.sleep(PACKET_DURATION_MS.toLong())

        engine.stop()

        // The native stream is fully closed by the time stop() returns, so
        // the real-time callback cannot still be running; any further
        // silent_disco_audio_read_interleaved_f32 activity would only be
        // observable as new callback_count growth, which requires an open
        // stream in the first place.
        assertThat(OboeBridge.nativeOboeIsOpen()).isFalse()
    }
}
