package com.ekkus.silentdisco.core.transport

import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import java.nio.ByteBuffer
import java.nio.charset.StandardCharsets

internal interface MessageCodec<T> {
    fun encode(value: T): ByteArray
    fun decode(bytes: ByteArray): T
}

/**
 * Kotlin's binary encoding of [AudioPacket]. Retained after the Wi-Fi Direct
 * socket transport was removed (Block 20) solely so
 * `RustMigrationCompatibilityFixtureTest` can keep asserting that Kotlin's
 * packetizer output matches the shared cross-language fixture byte-for-byte.
 */
internal object AudioPacketCodec : MessageCodec<AudioPacket> {
    override fun encode(value: AudioPacket): ByteArray {
        val sessionIdBytes = value.sessionId.value.encodeToByteArray()
        val streamIdBytes = value.streamId.value.encodeToByteArray()
        val codecBytes = value.codec.encodeToByteArray()
        val payload = value.payload
        val buffer = ByteBuffer.allocate(
            Int.SIZE_BYTES * 10 +
                Long.SIZE_BYTES * 3 +
                sessionIdBytes.size +
                streamIdBytes.size +
                codecBytes.size +
                payload.size,
        )
        buffer.putInt(value.version)
        putBytes(buffer, sessionIdBytes)
        putBytes(buffer, streamIdBytes)
        buffer.putLong(value.sequenceNumber)
        putBytes(buffer, codecBytes)
        buffer.putInt(value.sampleRate)
        buffer.putInt(value.channelCount)
        buffer.putInt(value.samplesPerPacket)
        buffer.putLong(value.firstSampleIndex)
        buffer.putLong(value.hostPresentationTimeMs)
        putBytes(buffer, payload)
        buffer.putInt(value.checksum ?: Int.MIN_VALUE)
        return buffer.array()
    }

    override fun decode(bytes: ByteArray): AudioPacket {
        val buffer = ByteBuffer.wrap(bytes)
        return AudioPacket(
            version = buffer.int,
            sessionId = SessionId(readBytes(buffer).toString(StandardCharsets.UTF_8)),
            streamId = StreamId(readBytes(buffer).toString(StandardCharsets.UTF_8)),
            sequenceNumber = buffer.long,
            codec = readBytes(buffer).toString(StandardCharsets.UTF_8),
            sampleRate = buffer.int,
            channelCount = buffer.int,
            samplesPerPacket = buffer.int,
            firstSampleIndex = buffer.long,
            hostPresentationTimeMs = buffer.long,
            payload = readBytes(buffer),
            checksum = buffer.int.takeUnless { it == Int.MIN_VALUE },
        )
    }

    private fun putBytes(buffer: ByteBuffer, bytes: ByteArray) {
        buffer.putInt(bytes.size)
        buffer.put(bytes)
    }

    private fun readBytes(buffer: ByteBuffer): ByteArray {
        val size = buffer.int
        return ByteArray(size).also(buffer::get)
    }
}
