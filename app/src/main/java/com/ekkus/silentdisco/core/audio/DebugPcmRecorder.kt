package com.ekkus.silentdisco.core.audio

import java.io.File
import java.io.RandomAccessFile
import java.nio.ByteBuffer
import java.nio.ByteOrder

private const val WAV_HEADER_BYTES = 44
private const val BITS_PER_SAMPLE = 16
private const val PCM_FORMAT_TAG: Short = 1

/**
 * Records the exact PCM16LE bytes handed to [PlaybackEngine.write] to a real
 * WAV file, in order, as they are sent -- not a re-synthesis of what
 * *should* have played. Pulling this file off the device and comparing it
 * against the known source audio is an objective ground truth beyond a
 * subjective description of what playback sounded like: it shows precisely
 * which samples the app itself believed it was playing, including any
 * gaps (concealment silence) or corruption already present at that point.
 *
 * Diagnostic-only: not part of the production playback path, and safe to
 * leave wired in as long as this project remains a viability PoC -- it adds
 * no behavior beyond appending bytes already computed for real playback.
 */
class DebugPcmRecorder(private val file: File) {
    private var output: RandomAccessFile? = null
    private var sampleRate = 0
    private var channelCount = 0
    private var dataBytes = 0L

    fun start(sampleRate: Int, channelCount: Int) {
        this.sampleRate = sampleRate
        this.channelCount = channelCount
        dataBytes = 0
        file.parentFile?.mkdirs()
        val opened = RandomAccessFile(file, "rw")
        opened.setLength(0)
        opened.write(ByteArray(WAV_HEADER_BYTES))
        output = opened
    }

    fun append(payload: ByteArray) {
        val active = output ?: return
        active.write(payload)
        dataBytes += payload.size
    }

    /** Finalizes the WAV header now that the real data length is known, and closes the file. */
    fun finish() {
        val active = output ?: return
        output = null
        active.seek(0)
        active.write(buildWavHeader(sampleRate, channelCount, dataBytes))
        active.close()
    }
}

private fun buildWavHeader(sampleRate: Int, channelCount: Int, dataBytes: Long): ByteArray {
    val blockAlign = channelCount * (BITS_PER_SAMPLE / 8)
    val byteRate = sampleRate * blockAlign
    val buffer = ByteBuffer.allocate(WAV_HEADER_BYTES).order(ByteOrder.LITTLE_ENDIAN)
    buffer.put("RIFF".toByteArray(Charsets.US_ASCII))
    buffer.putInt((36 + dataBytes).toInt())
    buffer.put("WAVE".toByteArray(Charsets.US_ASCII))
    buffer.put("fmt ".toByteArray(Charsets.US_ASCII))
    buffer.putInt(16)
    buffer.putShort(PCM_FORMAT_TAG)
    buffer.putShort(channelCount.toShort())
    buffer.putInt(sampleRate)
    buffer.putInt(byteRate)
    buffer.putShort(blockAlign.toShort())
    buffer.putShort(BITS_PER_SAMPLE.toShort())
    buffer.put("data".toByteArray(Charsets.US_ASCII))
    buffer.putInt(dataBytes.toInt())
    return buffer.array()
}
