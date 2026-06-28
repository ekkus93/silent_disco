package com.ekkus.silentdisco.core.audio

import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.google.common.truth.Truth.assertThat
import org.junit.Test

private class FailingPlaybackEngine : PlaybackEngine {
    override fun start(format: AudioFormatSpec): String = "fake"
    override fun write(frame: PlaybackFrame): Long = error("Injected write failure")
    override fun setVolume(value: Float) = Unit
    override fun playbackPositionMs(frame: PlaybackFrame): Long = 0L
    override fun stop() = Unit
}

private class FailingStartPlaybackEngine : PlaybackEngine {
    override fun start(format: AudioFormatSpec): String = error("Injected start failure")
    override fun write(frame: PlaybackFrame): Long = 0L
    override fun setVolume(value: Float) = Unit
    override fun playbackPositionMs(frame: PlaybackFrame): Long = 0L
    override fun stop() = Unit
}

private class RecordingPlaybackEngine : PlaybackEngine {
    val starts = mutableListOf<AudioFormatSpec>()
    val writes = mutableListOf<PlaybackFrame>()
    val volumes = mutableListOf<Float>()

    override fun start(format: AudioFormatSpec): String {
        starts += format
        return "recording-engine"
    }
    override fun write(frame: PlaybackFrame): Long {
        writes += frame
        return frame.packet.payload.size.toLong()
    }
    override fun setVolume(value: Float) { volumes += value }
    override fun playbackPositionMs(frame: PlaybackFrame): Long = 0L
    override fun stop() = Unit
}

private fun testFrame(payloadSize: Int = 3840) = PlaybackFrame(
    packet = AudioPacket(
        version = 1,
        sessionId = SessionId("session"),
        streamId = StreamId("stream"),
        sequenceNumber = 1L,
        codec = "pcm16le",
        sampleRate = 48_000,
        channelCount = 2,
        samplesPerPacket = 960,
        firstSampleIndex = 0L,
        hostPresentationTimeMs = 0L,
        payload = ByteArray(payloadSize),
    ),
    localDeadlineMs = 0L,
)

class PlaybackEngineInterfaceTest {

    @Test
    fun failingEngine_start_throwsOnWrite() {
        val engine = FailingPlaybackEngine()
        engine.start()
        val frame = testFrame()
        org.junit.Assert.assertThrows(IllegalStateException::class.java) {
            engine.write(frame)
        }
    }

    @Test
    fun failingStartEngine_start_throwsImmediately() {
        val engine = FailingStartPlaybackEngine()
        org.junit.Assert.assertThrows(IllegalStateException::class.java) {
            engine.start()
        }
    }

    @Test
    fun recordingEngine_capturesStartAndWrite() {
        val engine = RecordingPlaybackEngine()
        val format = AudioFormatSpec(sampleRate = 48_000, channelCount = 2)
        engine.start(format)

        assertThat(engine.starts).hasSize(1)
        assertThat(engine.starts[0].sampleRate).isEqualTo(48_000)
        assertThat(engine.starts[0].channelCount).isEqualTo(2)
    }

    @Test
    fun recordingEngine_capturesSetVolume() {
        val engine = RecordingPlaybackEngine()
        engine.setVolume(0.5f)
        engine.setVolume(0.8f)
        assertThat(engine.volumes).containsExactly(0.5f, 0.8f).inOrder()
    }

    @Test
    fun audioTrackPlaybackEngine_writeBeforeStart_throws() {
        val engine = AudioTrackPlaybackEngine()
        val frame = testFrame()
        org.junit.Assert.assertThrows(IllegalStateException::class.java) {
            engine.write(frame)
        }
    }

    @Test
    fun audioTrackPlaybackEngine_stop_doesNotThrowWhenNotStarted() {
        val engine = AudioTrackPlaybackEngine()
        engine.stop()
    }
}
