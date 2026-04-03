package com.ekkus.silentdisco.core.audio

import android.content.ContentResolver
import android.media.AudioFormat
import android.media.MediaCodec
import android.media.MediaExtractor
import android.media.MediaFormat
import android.net.Uri
import android.os.Build
import android.provider.OpenableColumns
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import java.nio.ByteBuffer
import kotlin.math.abs
import kotlin.math.roundToInt

data class AudioDecodeResult(
    val format: AudioFormatSpec,
    val chunks: List<DecodedAudioChunk>,
    val totalFrameCount: Long,
    val reachedEndOfFile: Boolean,
)

class AudioFileAccessException(message: String, cause: Throwable? = null) : Exception(message, cause)

class AudioFileDecoder(
    private val contentResolver: ContentResolver,
) {
    fun describe(uri: Uri, fallbackName: String? = null, fallbackMimeType: String? = null): SelectedAudioFile {
        val projection = arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE)
        val cursor = contentResolver.query(uri, projection, null, null, null)
            ?: throw AudioFileAccessException("Unable to query audio file metadata")
        cursor.use {
            if (!it.moveToFirst()) {
                throw AudioFileAccessException("Audio file metadata is empty")
            }
            val nameIndex = it.getColumnIndex(OpenableColumns.DISPLAY_NAME)
            val sizeIndex = it.getColumnIndex(OpenableColumns.SIZE)
            val displayName = if (nameIndex >= 0) it.getString(nameIndex) else fallbackName ?: "audio-file"
            val sizeBytes = if (sizeIndex >= 0 && !it.isNull(sizeIndex)) it.getLong(sizeIndex) else null
            val mimeType = contentResolver.getType(uri) ?: fallbackMimeType
            if (mimeType != null && !mimeType.startsWith("audio/")) {
                throw AudioFileAccessException("Selected file is not recognized as audio: $mimeType")
            }
            return SelectedAudioFile(
                uri = uri,
                displayName = displayName,
                mimeType = mimeType,
                sizeBytes = sizeBytes,
            )
        }
    }

    fun decode(selectedAudio: SelectedAudioFile, packetDurationMs: Int = 20): AudioDecodeResult {
        val extractor = MediaExtractor()
        val assetFileDescriptor = try {
            contentResolver.openAssetFileDescriptor(selectedAudio.uri, "r")
                ?: throw AudioFileAccessException("Failed to open selected audio file")
        } catch (error: Throwable) {
            throw AudioFileAccessException("Failed to open audio file for decoding", error)
        }
        try {
            extractor.setDataSource(assetFileDescriptor.fileDescriptor, assetFileDescriptor.startOffset, assetFileDescriptor.length)
            val trackIndex = (0 until extractor.trackCount).firstOrNull { index ->
                extractor.getTrackFormat(index).getString(MediaFormat.KEY_MIME)?.startsWith("audio/") == true
            } ?: throw AudioFileAccessException("No audio track found in selected file")
            extractor.selectTrack(trackIndex)
            val inputFormat = extractor.getTrackFormat(trackIndex)
            val mimeType = inputFormat.getString(MediaFormat.KEY_MIME)
                ?: throw AudioFileAccessException("Audio track MIME type unavailable")
            val codec = MediaCodec.createDecoderByType(mimeType)
            codec.configure(inputFormat, null, null, 0)
            codec.start()
            try {
                return decodeWithCodec(extractor, codec, inputFormat, packetDurationMs)
            } finally {
                codec.stop()
                codec.release()
            }
        } finally {
            assetFileDescriptor.close()
            extractor.release()
        }
    }

    private fun decodeWithCodec(
        extractor: MediaExtractor,
        codec: MediaCodec,
        inputFormat: MediaFormat,
        packetDurationMs: Int,
    ): AudioDecodeResult {
        val bufferInfo = MediaCodec.BufferInfo()
        val outputPcm = ArrayList<Byte>()
        var inputDone = false
        var outputDone = false
        var outputFormat = inputFormat

        while (!outputDone) {
            if (!inputDone) {
                val inputIndex = codec.dequeueInputBuffer(10_000)
                if (inputIndex >= 0) {
                    val inputBuffer = codec.getInputBuffer(inputIndex) ?: ByteBuffer.allocate(0)
                    inputBuffer.clear()
                    val sampleSize = extractor.readSampleData(inputBuffer, 0)
                    if (sampleSize < 0) {
                        codec.queueInputBuffer(inputIndex, 0, 0, 0, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
                        inputDone = true
                    } else {
                        codec.queueInputBuffer(
                            inputIndex,
                            0,
                            sampleSize,
                            extractor.sampleTime,
                            0,
                        )
                        extractor.advance()
                    }
                }
            }

            val outputIndex = codec.dequeueOutputBuffer(bufferInfo, 10_000)
            when {
                outputIndex >= 0 -> {
                    val outputBuffer = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.LOLLIPOP) {
                        codec.getOutputBuffer(outputIndex)
                    } else {
                        @Suppress("DEPRECATION")
                        codec.outputBuffers[outputIndex]
                    } ?: ByteBuffer.allocate(0)
                    if (bufferInfo.size > 0) {
                        val bytes = ByteArray(bufferInfo.size)
                        outputBuffer.position(bufferInfo.offset)
                        outputBuffer.limit(bufferInfo.offset + bufferInfo.size)
                        outputBuffer.get(bytes)
                        bytes.forEach(outputPcm::add)
                    }
                    codec.releaseOutputBuffer(outputIndex, false)
                    if (bufferInfo.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) {
                        outputDone = true
                    }
                }
                outputIndex == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                    outputFormat = codec.outputFormat
                }
            }
        }

        val normalized = normalizeToPcm16Stereo48k(outputPcm.toByteArray(), outputFormat, packetDurationMs)
        return normalized
    }

    private fun normalizeToPcm16Stereo48k(
        sourcePcm: ByteArray,
        sourceFormat: MediaFormat,
        packetDurationMs: Int,
    ): AudioDecodeResult {
        val sourceRate = sourceFormat.getInteger(MediaFormat.KEY_SAMPLE_RATE)
        val sourceChannels = sourceFormat.getInteger(MediaFormat.KEY_CHANNEL_COUNT)
        val pcmEncoding = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N && sourceFormat.containsKey(MediaFormat.KEY_PCM_ENCODING)) {
            sourceFormat.getInteger(MediaFormat.KEY_PCM_ENCODING)
        } else {
            AudioFormat.ENCODING_PCM_16BIT
        }
        val bytesPerSample = when (pcmEncoding) {
            AudioFormat.ENCODING_PCM_16BIT -> 2
            AudioFormat.ENCODING_PCM_FLOAT -> 4
            else -> 2
        }
        val sourceFrameSize = sourceChannels * bytesPerSample
        if (sourceFrameSize <= 0 || sourcePcm.isEmpty()) {
            throw AudioFileAccessException("Decoded audio stream is empty")
        }
        val sourceFrameCount = sourcePcm.size / sourceFrameSize
        val sourceSamples = FloatArray(sourceFrameCount * sourceChannels)
        val sourceBuffer = ByteBuffer.wrap(sourcePcm).order(java.nio.ByteOrder.LITTLE_ENDIAN)
        for (frame in 0 until sourceFrameCount) {
            for (channel in 0 until sourceChannels) {
                val sample = if (bytesPerSample == 2) {
                    sourceBuffer.short.toFloat() / Short.MAX_VALUE.toFloat()
                } else {
                    sourceBuffer.int.toFloat() / Int.MAX_VALUE.toFloat()
                }
                sourceSamples[frame * sourceChannels + channel] = sample
            }
        }

        val targetSpec = AudioFormatSpec()
        val resampleRatio = targetSpec.sampleRate.toDouble() / sourceRate.toDouble()
        val targetFrameCount = (sourceFrameCount * resampleRatio).roundToInt().coerceAtLeast(1)
        val targetPcm = ByteBuffer.allocate(targetFrameCount * targetSpec.bytesPerFrame)
            .order(java.nio.ByteOrder.LITTLE_ENDIAN)
        for (frame in 0 until targetFrameCount) {
            val sourcePosition = frame / resampleRatio
            val baseFrame = sourcePosition.toInt().coerceIn(0, sourceFrameCount - 1)
            val nextFrame = (baseFrame + 1).coerceAtMost(sourceFrameCount - 1)
            val blend = (sourcePosition - baseFrame).toFloat()
            val mixedChannels = FloatArray(targetSpec.channelCount)
            for (targetChannel in 0 until targetSpec.channelCount) {
                val sourceChannel = when {
                    sourceChannels == 1 -> 0
                    targetChannel < sourceChannels -> targetChannel
                    else -> sourceChannels - 1
                }
                val start = sourceSamples[baseFrame * sourceChannels + sourceChannel]
                val end = sourceSamples[nextFrame * sourceChannels + sourceChannel]
                val interpolated = start + (end - start) * blend
                mixedChannels[targetChannel] = interpolated
            }
            if (sourceChannels == 1 && targetSpec.channelCount == 2) {
                mixedChannels[1] = mixedChannels[0]
            }
            mixedChannels.forEach { sample ->
                val clamped = sample.coerceIn(-1f, 1f)
                targetPcm.putShort((clamped * Short.MAX_VALUE).roundToInt().toShort())
            }
        }

        val packetFrames = targetSpec.sampleRate * packetDurationMs / 1_000
        val packetBytes = packetFrames * targetSpec.bytesPerFrame
        val normalizedBytes = targetPcm.array()
        val chunks = normalizedBytes.toList()
            .chunked(packetBytes)
            .mapIndexed { index, bytes ->
                DecodedAudioChunk(
                    pcm16Le = bytes.toByteArray(),
                    firstSampleIndex = index.toLong() * packetFrames,
                    frameCount = bytes.size / targetSpec.bytesPerFrame,
                )
            }
            .filter { it.frameCount > 0 }

        val measuredPeak = chunks
            .flatMap { chunk ->
                ByteBuffer.wrap(chunk.pcm16Le).order(java.nio.ByteOrder.LITTLE_ENDIAN).run {
                    buildList {
                        while (remaining() >= 2) add(abs(short.toInt()))
                    }
                }
            }
            .maxOrNull() ?: 0
        if (measuredPeak == 0) {
            throw AudioFileAccessException("Decoded audio contains only silence or unreadable samples")
        }

        return AudioDecodeResult(
            format = targetSpec,
            chunks = chunks,
            totalFrameCount = targetFrameCount.toLong(),
            reachedEndOfFile = true,
        )
    }
}
