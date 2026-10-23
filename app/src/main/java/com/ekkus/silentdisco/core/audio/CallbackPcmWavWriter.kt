package com.ekkus.silentdisco.core.audio

import java.io.Closeable
import java.io.File
import java.io.RandomAccessFile
import kotlin.math.roundToInt

/**
 * Writes interleaved stereo float32 PCM captured *after* the native callback
 * returns, not on Oboe's real-time thread. Reference WAVs must use the same
 * canonical decode, gain and quantization policy.
 *
 * This sink is intentionally bounded: exceeding the maximum fails loudly
 * rather than producing an apparently complete but truncated diagnostic.
 */
internal class CallbackPcmWavWriter(
    val file: File,
    val sampleRate: Int = 48_000,
    private val maxFrames: Long = 48_000L * 300L,
) : Closeable {
    private val output = RandomAccessFile(file, "rw")
    private var samplesWritten = 0L
    private var finished = false

    init {
        require(sampleRate > 0 && maxFrames > 0)
        require(maxFrames <= (Int.MAX_VALUE - 44L) / 4L)
        output.setLength(0)
        output.writeBytes("RIFF")
        output.writeLe32(36)
        output.writeBytes("WAVE")
        output.writeBytes("fmt ")
        output.writeLe32(16)
        output.writeLe16(1)
        output.writeLe16(2)
        output.writeLe32(sampleRate)
        output.writeLe32(sampleRate * 4)
        output.writeLe16(4)
        output.writeLe16(16)
        output.writeBytes("data")
        output.writeLe32(0)
    }

    @Synchronized
    fun append(samples: List<Float>) {
        check(!finished) { "callback WAV capture was already finished" }
        require(samples.size % 2 == 0) { "an interleaved stereo frame must be complete" }
        val frames = samples.size / 2
        check(samplesWritten + frames <= maxFrames) { "callback capture reached its frame limit" }
        val bytes = ByteArray(samples.size * 2)
        for ((index, sample) in samples.withIndex()) {
            require(sample.isFinite()) { "callback PCM contains a non-finite sample" }
            val pcm = (sample.coerceIn(-1f, 1f) * 32768f)
                .roundToInt().coerceIn(Short.MIN_VALUE.toInt(), Short.MAX_VALUE.toInt())
            bytes[index * 2] = pcm.toByte()
            bytes[index * 2 + 1] = (pcm ushr 8).toByte()
        }
        output.write(bytes)
        samplesWritten += frames
    }

    @Synchronized
    fun finish(): Long {
        if (finished) return samplesWritten
        finished = true
        val bytes = samplesWritten * 4L
        output.seek(4)
        output.writeLe32((36L + bytes).toInt())
        output.seek(40)
        output.writeLe32(bytes.toInt())
        output.close()
        return samplesWritten
    }

    override fun close() {
        finish()
    }
}

private fun RandomAccessFile.writeLe16(value: Int) {
    writeByte(value and 0xff)
    writeByte((value ushr 8) and 0xff)
}

private fun RandomAccessFile.writeLe32(value: Int) {
    writeLe16(value and 0xffff)
    writeLe16(value ushr 16)
}
