package com.ekkus.silentdisco.core.audio

import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.zip.CRC32

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

data class BufferedAudioPacket(
    val packet: AudioPacket,
    val scheduledLocalTimeMs: Long,
)

data class PacketizationStats(
    val packetCount: Int,
    val averagePayloadBytes: Double,
    val maxPayloadBytes: Int,
)

class AudioPacketBuffer(
    private val startupTargetMs: Long = 400,
) {
    private val packets = sortedMapOf<Long, BufferedAudioPacket>()
    private var lastEmittedSequence: Long? = null

    fun insert(packet: BufferedAudioPacket) {
        packets[packet.packet.sequenceNumber] = packet
    }

    fun isReady(): Boolean {
        if (packets.isEmpty()) return false
        val first = packets.values.first()
        val last = packets.values.last()
        return last.scheduledLocalTimeMs - first.scheduledLocalTimeMs >= startupTargetMs
    }

    fun popReady(nowLocalTimeMs: Long): BufferedAudioPacket? {
        val firstEntry = packets.entries.firstOrNull() ?: return null
        if (firstEntry.value.scheduledLocalTimeMs > nowLocalTimeMs) return null
        lastEmittedSequence = firstEntry.key
        return packets.remove(firstEntry.key)
    }

    fun missingSequenceCount(): Int {
        val last = lastEmittedSequence ?: return 0
        val next = packets.keys.firstOrNull() ?: return 0
        return (next - last - 1).coerceAtLeast(0).toInt()
    }

    fun depthMs(): Long {
        if (packets.isEmpty()) return 0
        return packets.values.last().scheduledLocalTimeMs - packets.values.first().scheduledLocalTimeMs
    }

    fun validatePacketIdentity(packet: AudioPacket, sessionId: SessionId, streamId: StreamId): Boolean {
        return packet.sessionId == sessionId && packet.streamId == streamId
    }

    fun missingSequenceRanges(): List<LongRange> {
        val sortedKeys = packets.keys.toList()
        if (sortedKeys.size < 2) return emptyList()
        val gaps = mutableListOf<LongRange>()
        for (index in 1 until sortedKeys.size) {
            val previous = sortedKeys[index - 1]
            val current = sortedKeys[index]
            if (current > previous + 1) {
                gaps += (previous + 1)..(current - 1)
            }
        }
        return gaps
    }
}

object SilenceFiller {
    fun pcm16Le(frameCount: Int, format: AudioFormatSpec = AudioFormatSpec()): ByteArray {
        return ByteBuffer.allocate(frameCount * format.bytesPerFrame)
            .order(ByteOrder.LITTLE_ENDIAN)
            .array()
    }
}

fun List<AudioPacket>.packetizationStats(): PacketizationStats {
    if (isEmpty()) {
        return PacketizationStats(packetCount = 0, averagePayloadBytes = 0.0, maxPayloadBytes = 0)
    }
    val payloadSizes = map { it.payload.size }
    return PacketizationStats(
        packetCount = size,
        averagePayloadBytes = payloadSizes.average(),
        maxPayloadBytes = payloadSizes.maxOrNull() ?: 0,
    )
}
