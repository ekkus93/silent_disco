package com.ekkus.silentdisco.core.audio

import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import java.util.zip.CRC32

private const val EstimatedPacketHeaderBytes = 48

data class AudioFormatSpec(
    val sampleRate: Int = 48_000,
    val channelCount: Int = 2,
    val bytesPerSample: Int = 2,
) {
    val bytesPerFrame: Int = channelCount * bytesPerSample
}

data class DecodedAudioChunk(
    val pcm16Le: ByteArray,
    val firstSampleIndex: Long,
    val frameCount: Int,
)

class PcmPacketizer(
    private val sessionId: SessionId,
    private val streamId: StreamId,
    private val format: AudioFormatSpec = AudioFormatSpec(),
    private val packetDurationMs: Int = 20,
) {
    private val samplesPerChannelPerPacket = format.sampleRate * packetDurationMs / 1_000
    private val bytesPerPacket = samplesPerChannelPerPacket * format.bytesPerFrame

    fun packetize(chunk: DecodedAudioChunk, hostPresentationStartMs: Long): List<AudioPacket> {
        if (chunk.pcm16Le.isEmpty()) return emptyList()
        val packets = mutableListOf<AudioPacket>()
        var sequence = 0L
        var byteOffset = 0
        while (byteOffset < chunk.pcm16Le.size) {
            val end = minOf(byteOffset + bytesPerPacket, chunk.pcm16Le.size)
            val payload = chunk.pcm16Le.copyOfRange(byteOffset, end)
            val firstSampleIndex = chunk.firstSampleIndex + sequence * samplesPerChannelPerPacket
            val hostPresentationTimeMs = hostPresentationStartMs + sequence * packetDurationMs
            packets += AudioPacket(
                version = 1,
                sessionId = sessionId,
                streamId = streamId,
                sequenceNumber = sequence,
                codec = "pcm16le",
                sampleRate = format.sampleRate,
                channelCount = format.channelCount,
                samplesPerPacket = samplesPerChannelPerPacket,
                firstSampleIndex = firstSampleIndex,
                hostPresentationTimeMs = hostPresentationTimeMs,
                payload = payload,
                checksum = crc32(payload),
            )
            sequence += 1
            byteOffset = end
        }
        return packets
    }

    private fun crc32(payload: ByteArray): Int {
        val crc = CRC32()
        crc.update(payload)
        return crc.value.toInt()
    }
}

data class PacketizationStats(
    val packetCount: Int,
    val averagePayloadBytes: Double,
    val maxPayloadBytes: Int,
    val averagePacketBytes: Double,
    val maxPacketBytes: Int,
    val headerBytesPerPacket: Int,
    val overheadRatio: Double,
)

data class PacketBudgetValidation(
    val valid: Boolean,
    val averagePacketBytes: Int,
    val maxPacketBytes: Int,
    val overheadRatio: Double,
) {
    fun summary(): String =
        "avg=${averagePacketBytes}B, max=${maxPacketBytes}B, overhead=${"%.1f".format(overheadRatio * 100)}%"
}

fun List<AudioPacket>.packetizationStats(): PacketizationStats {
    if (isEmpty()) {
        return PacketizationStats(
            packetCount = 0,
            averagePayloadBytes = 0.0,
            maxPayloadBytes = 0,
            averagePacketBytes = 0.0,
            maxPacketBytes = 0,
            headerBytesPerPacket = EstimatedPacketHeaderBytes,
            overheadRatio = 0.0,
        )
    }
    val payloadSizes = map { it.payload.size }
    val packetSizes = map { it.estimatedWireBytes() }
    val averagePayload = payloadSizes.average()
    val averagePacket = packetSizes.average()
    return PacketizationStats(
        packetCount = size,
        averagePayloadBytes = averagePayload,
        maxPayloadBytes = payloadSizes.maxOrNull() ?: 0,
        averagePacketBytes = averagePacket,
        maxPacketBytes = packetSizes.maxOrNull() ?: 0,
        headerBytesPerPacket = EstimatedPacketHeaderBytes,
        overheadRatio = if (averagePacket == 0.0) 0.0 else (averagePacket - averagePayload) / averagePacket,
    )
}

fun AudioPacket.estimatedWireBytes(headerBytes: Int = EstimatedPacketHeaderBytes): Int = payload.size + headerBytes

fun List<AudioPacket>.validatePacketBudget(maxWireBytes: Int = 4_096): PacketBudgetValidation {
    val stats = packetizationStats()
    return PacketBudgetValidation(
        valid = stats.maxPacketBytes <= maxWireBytes,
        averagePacketBytes = stats.averagePacketBytes.toInt(),
        maxPacketBytes = stats.maxPacketBytes,
        overheadRatio = stats.overheadRatio,
    )
}
