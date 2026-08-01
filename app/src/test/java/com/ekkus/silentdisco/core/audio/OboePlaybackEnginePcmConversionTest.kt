package com.ekkus.silentdisco.core.audio

import com.google.common.truth.Truth.assertThat
import org.junit.Test

/**
 * Covers only the pure PCM16LE -> interleaved float32 conversion
 * [OboePlaybackEngine] uses before pushing frames into the Rust render
 * ring. The engine's actual Oboe/UniFFI-backed behavior requires a physical
 * device (native library, real audio hardware) and is validated by the
 * instrumented test, not here.
 */
class OboePlaybackEnginePcmConversionTest {

    private fun littleEndianBytes(vararg samples: Short): ByteArray {
        val bytes = ByteArray(samples.size * 2)
        samples.forEachIndexed { index, sample ->
            bytes[index * 2] = (sample.toInt() and 0xFF).toByte()
            bytes[index * 2 + 1] = ((sample.toInt() shr 8) and 0xFF).toByte()
        }
        return bytes
    }

    @Test
    fun `converts full scale positive and negative samples to the expected float range`() {
        val payload = littleEndianBytes(32767, -32768, 0)

        val samples = pcm16LeToFloat(payload, volume = 1.0f)

        assertThat(samples).hasLength(3)
        assertThat(samples[0]).isWithin(0.0001f).of(32767f / 32768f)
        assertThat(samples[1]).isWithin(0.0001f).of(-1.0f)
        assertThat(samples[2]).isWithin(0.0001f).of(0.0f)
    }

    @Test
    fun `applies volume as a linear gain factor`() {
        val payload = littleEndianBytes(16384)

        val fullVolume = pcm16LeToFloat(payload, volume = 1.0f)
        val halfVolume = pcm16LeToFloat(payload, volume = 0.5f)
        val muted = pcm16LeToFloat(payload, volume = 0.0f)

        assertThat(halfVolume[0]).isWithin(0.0001f).of(fullVolume[0] / 2f)
        assertThat(muted[0]).isWithin(0.0001f).of(0.0f)
    }

    @Test
    fun `preserves sample count and order across multiple frames`() {
        val payload = littleEndianBytes(100, -100, 200, -200)

        val samples = pcm16LeToFloat(payload, volume = 1.0f)

        assertThat(samples).hasLength(4)
        assertThat(samples[0]).isGreaterThan(0f)
        assertThat(samples[1]).isLessThan(0f)
        assertThat(samples[2]).isGreaterThan(samples[0])
        assertThat(samples[3]).isLessThan(samples[1])
    }

    @Test
    fun `empty payload converts to an empty sample array`() {
        val samples = pcm16LeToFloat(ByteArray(0), volume = 1.0f)

        assertThat(samples).isEmpty()
    }
}
