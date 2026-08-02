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

data class BufferedAudioPacket(
    val packet: AudioPacket,
    val scheduledLocalTimeMs: Long,
)

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

/**
 * Buffers packets between the network-reception coroutine ([insert]) and the
 * playback coroutine ([popReady]/[peekFirst]/[isReady]) -- two independent
 * `Dispatchers.IO` coroutines that run concurrently on different threads, not
 * a single confined thread. `TreeMap` (the backing type of [sortedMapOf]) is
 * not thread-safe, so unsynchronized cross-thread access here previously
 * crashed the whole process with a `ConcurrentModificationException` inside
 * [popReady] whenever [insert] mutated the map mid-iteration on another
 * thread. All access is synchronized on this instance.
 */
class AudioPacketBuffer(
    private val startupTargetMs: Long = 400,
) {
    private val packets = sortedMapOf<Long, BufferedAudioPacket>()
    private var lastEmittedSequence: Long? = null

    fun insert(packet: BufferedAudioPacket) {
        synchronized(this) {
            packets[packet.packet.sequenceNumber] = packet
        }
    }

    fun isReady(): Boolean = synchronized(this) {
        if (packets.isEmpty()) return@synchronized false
        val first = packets.values.first()
        val last = packets.values.last()
        last.scheduledLocalTimeMs - first.scheduledLocalTimeMs >= startupTargetMs
    }

    fun popReady(nowLocalTimeMs: Long): BufferedAudioPacket? = synchronized(this) {
        val firstEntry = packets.entries.firstOrNull() ?: return@synchronized null
        if (firstEntry.value.scheduledLocalTimeMs > nowLocalTimeMs) return@synchronized null
        lastEmittedSequence = firstEntry.key
        packets.remove(firstEntry.key)
    }

    fun peekFirst(): BufferedAudioPacket? = synchronized(this) { packets.values.firstOrNull() }

    /**
     * Drains every buffered packet in sequence order, ignoring each one's
     * scheduled deadline. Used when a stream stops: anything still buffered
     * here already arrived over the network in time -- it is real content,
     * not backlog -- so it should be played out, not silently discarded.
     */
    fun drainAll(): List<BufferedAudioPacket> = synchronized(this) {
        val drained = packets.values.toList()
        packets.clear()
        drained
    }

    fun missingSequenceCount(): Int = synchronized(this) {
        val last = lastEmittedSequence ?: return@synchronized 0
        val next = packets.keys.firstOrNull() ?: return@synchronized 0
        (next - last - 1).coerceAtLeast(0).toInt()
    }

    fun depthMs(): Long = synchronized(this) {
        if (packets.isEmpty()) return@synchronized 0
        packets.values.last().scheduledLocalTimeMs - packets.values.first().scheduledLocalTimeMs
    }

    fun validatePacketIdentity(packet: AudioPacket, sessionId: SessionId, streamId: StreamId): Boolean {
        return packet.sessionId == sessionId && packet.streamId == streamId
    }

    fun missingSequenceRanges(): List<LongRange> = synchronized(this) {
        val sortedKeys = packets.keys.toList()
        if (sortedKeys.size < 2) return@synchronized emptyList()
        val gaps = mutableListOf<LongRange>()
        for (index in 1 until sortedKeys.size) {
            val previous = sortedKeys[index - 1]
            val current = sortedKeys[index]
            if (current > previous + 1) {
                gaps += (previous + 1)..(current - 1)
            }
        }
        gaps
    }
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
