package com.ekkus.silentdisco

import androidx.test.platform.app.InstrumentationRegistry
import com.ekkus.silentdisco.core.rust.HostPlaylistStreamController
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistFrame
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistSource
import com.google.common.truth.Truth.assertThat
import java.io.File
import java.util.UUID
import org.junit.Test

class HostPlaylistNativeStreamTest {
    @Test
    fun generatedAndroidBindingRetargetsThePrebufferBeforeItsBoundary() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val folder = File(context.cacheDir, "playlist-retarget-${UUID.randomUUID()}").apply { mkdirs() }
        val current = File(folder, "current.wav").apply { writeBytes(wavPcm16(48_000, 2, 48_000, 3_000)) }
        val stale = File(folder, "stale.wav").apply { writeBytes(wavPcm16(48_000, 2, 480, 4_000)) }
        val replacement = File(folder, "replacement.wav").apply { writeBytes(wavPcm16(48_000, 2, 480, 6_000)) }
        val stream = HostPlaylistStreamController.open(
            sessionId = "android-retarget-session",
            streamId = "android-retarget-stream",
            hostStartTimeMs = 5_000uL,
            sources = listOf(
                FfiPlaylistSource("android-current", "Current", current.absolutePath),
                FfiPlaylistSource("android-stale", "Stale", stale.absolutePath),
            ),
        )

        try {
            stream.replaceUpcoming(
                listOf(FfiPlaylistSource("android-replacement", "Replacement", replacement.absolutePath)),
            )
            val boundaries = mutableListOf<String>()
            var ended = false
            val deadline = System.nanoTime() + 10_000_000_000L
            while (!ended && System.nanoTime() < deadline) {
                when (val frame = stream.nextFrame(500uL)) {
                    null -> Unit
                    FfiPlaylistFrame.End -> ended = true
                    is FfiPlaylistFrame.TrackBoundary -> boundaries += frame.entryId
                    is FfiPlaylistFrame.Audio -> Unit
                }
            }
            assertThat(ended).isTrue()
            assertThat(boundaries).containsExactly("android-replacement")
        } finally {
            runCatching { stream.cancel() }
            stream.close()
            folder.deleteRecursively()
        }
    }

    @Test
    fun generatedAndroidBindingStreamsDifferentFormatsOnOneContinuousTimeline() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val folder = File(context.cacheDir, "playlist-test-${UUID.randomUUID()}").apply { mkdirs() }
        val first = File(folder, "first.wav").apply { writeBytes(wavPcm16(48_000, 2, 240, 3_000)) }
        val second = File(folder, "second.wav").apply { writeBytes(wavPcm16(44_100, 1, 441, 6_000)) }
        val stream = HostPlaylistStreamController.open(
            sessionId = "android-playlist-session",
            streamId = "android-playlist-stream",
            hostStartTimeMs = 5_000uL,
            sources = listOf(
                FfiPlaylistSource("android-first", "First song", first.absolutePath),
                FfiPlaylistSource("android-second", "Second song", second.absolutePath),
            ),
        )

        try {
            assertThat(stream.streamStart.currentEntryId).isEqualTo("android-first")
            assertThat(stream.streamStart.currentDisplayName).isEqualTo("First song")
            val packetSequences = mutableListOf<ULong>()
            val packetStarts = mutableListOf<Long>()
            val packetFrames = mutableListOf<Long>()
            var framePosition = 0
            var boundaryPosition: Int? = null
            var nextTrackAudioPosition: Int? = null
            var boundarySampleIndex: Long? = null
            var ended = false
            val deadline = System.nanoTime() + 10_000_000_000L

            while (!ended && System.nanoTime() < deadline) {
                when (val frame = stream.nextFrame(500uL)) {
                    null -> Unit
                    FfiPlaylistFrame.End -> ended = true
                    is FfiPlaylistFrame.TrackBoundary -> {
                        assertThat(frame.entryId).isEqualTo("android-second")
                        boundarySampleIndex = frame.firstSampleIndex.toLong()
                        boundaryPosition = framePosition
                        framePosition += 1
                    }
                    is FfiPlaylistFrame.Audio -> {
                        if (boundaryPosition != null && nextTrackAudioPosition == null) {
                            nextTrackAudioPosition = framePosition
                        }
                        packetSequences += frame.sequence
                        packetStarts += frame.firstSampleIndex.toLong()
                        packetFrames += frame.payload.size.toLong() / 4L
                        framePosition += 1
                    }
                }
            }

            assertThat(ended).isTrue()
            assertThat(boundarySampleIndex).isEqualTo(240L)
            assertThat(boundaryPosition).isLessThan(nextTrackAudioPosition)
            assertThat(packetSequences.first()).isEqualTo(0uL)
            assertThat(packetSequences.zipWithNext().all { (firstSequence, nextSequence) ->
                nextSequence == firstSequence + 1uL
            }).isTrue()
            assertThat(packetStarts.first()).isEqualTo(0L)
            packetStarts.indices.drop(1).forEach { index ->
                assertThat(packetStarts[index]).isEqualTo(packetStarts[index - 1] + packetFrames[index - 1])
            }
            assertThat(packetStarts.last() + packetFrames.last()).isEqualTo(720L)
        } finally {
            runCatching { stream.cancel() }
            stream.close()
            folder.deleteRecursively()
        }
    }

    private fun wavPcm16(sampleRate: Int, channels: Int, frames: Int, sample: Short): ByteArray {
        val dataBytes = frames * channels * 2
        val output = ByteArray(44 + dataBytes)
        fun ascii(offset: Int, value: String) {
            value.encodeToByteArray().copyInto(output, offset)
        }
        fun u16(offset: Int, value: Int) {
            output[offset] = value.toByte()
            output[offset + 1] = (value shr 8).toByte()
        }
        fun u32(offset: Int, value: Int) {
            repeat(4) { index -> output[offset + index] = (value shr (index * 8)).toByte() }
        }
        ascii(0, "RIFF")
        u32(4, 36 + dataBytes)
        ascii(8, "WAVEfmt ")
        u32(16, 16)
        u16(20, 1)
        u16(22, channels)
        u32(24, sampleRate)
        u32(28, sampleRate * channels * 2)
        u16(32, channels * 2)
        u16(34, 16)
        ascii(36, "data")
        u32(40, dataBytes)
        var offset = 44
        repeat(frames * channels) {
            output[offset++] = sample.toByte()
            output[offset++] = (sample.toInt() shr 8).toByte()
        }
        return output
    }
}
