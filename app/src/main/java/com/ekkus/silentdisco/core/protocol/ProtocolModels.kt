package com.ekkus.silentdisco.core.protocol

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
@JvmInline
value class SessionId(val value: String)

@Serializable
@JvmInline
value class StreamId(val value: String)

@Serializable
data class DeviceIdentity(
    val deviceId: String,
    val displayName: String,
)

@Serializable
sealed interface ControlMessage {
    val version: Int
    val sessionId: SessionId

    @Serializable
    @SerialName("hello")
    data class Hello(
        override val version: Int,
        override val sessionId: SessionId,
        val sessionName: String,
        val hostName: String,
        val approvalRequired: Boolean,
    ) : ControlMessage

    @Serializable
    @SerialName("join_request")
    data class JoinRequest(
        override val version: Int,
        override val sessionId: SessionId,
        val device: DeviceIdentity,
        val inviteCode: String? = null,
    ) : ControlMessage

    @Serializable
    @SerialName("join_approval")
    data class JoinApproval(
        override val version: Int,
        override val sessionId: SessionId,
        val listenerId: String,
        val trustedForFuture: Boolean,
    ) : ControlMessage

    @Serializable
    @SerialName("join_rejection")
    data class JoinRejection(
        override val version: Int,
        override val sessionId: SessionId,
        val listenerId: String,
        val reason: String,
    ) : ControlMessage

    @Serializable
    @SerialName("heartbeat")
    data class Heartbeat(
        override val version: Int,
        override val sessionId: SessionId,
        val listenerId: String,
        val sentAtElapsedMs: Long,
    ) : ControlMessage

    @Serializable
    @SerialName("stream_start")
    data class StreamStart(
        override val version: Int,
        override val sessionId: SessionId,
        val streamId: StreamId,
        val hostStartTimeMs: Long,
        val sampleRate: Int,
        val channels: Int,
        val samplesPerPacket: Int,
    ) : ControlMessage

    @Serializable
    @SerialName("pause")
    data class Pause(
        override val version: Int,
        override val sessionId: SessionId,
        val streamId: StreamId,
        val hostPauseTimeMs: Long,
    ) : ControlMessage

    @Serializable
    @SerialName("stop")
    data class Stop(
        override val version: Int,
        override val sessionId: SessionId,
        val streamId: StreamId,
        val hostStopTimeMs: Long,
    ) : ControlMessage

    @Serializable
    @SerialName("disconnect")
    data class Disconnect(
        override val version: Int,
        override val sessionId: SessionId,
        val listenerId: String,
        val reason: String,
    ) : ControlMessage

    @Serializable
    @SerialName("resync_notice")
    data class ResyncNotice(
        override val version: Int,
        override val sessionId: SessionId,
        val listenerId: String,
        val reason: String,
    ) : ControlMessage
}

@Serializable
data class SyncRequestPacket(
    val version: Int,
    val sessionId: SessionId,
    val correlationId: Long,
    val t1ListenerSendElapsedMs: Long,
)

@Serializable
data class SyncResponsePacket(
    val version: Int,
    val sessionId: SessionId,
    val correlationId: Long,
    val t1ListenerSendElapsedMs: Long,
    val t2HostReceiveElapsedMs: Long,
    val t3HostSendElapsedMs: Long,
)

@Serializable
data class AudioPacket(
    val version: Int,
    val sessionId: SessionId,
    val streamId: StreamId,
    val sequenceNumber: Long,
    val codec: String,
    val sampleRate: Int,
    val channelCount: Int,
    val samplesPerPacket: Int,
    val firstSampleIndex: Long,
    val hostPresentationTimeMs: Long,
    val payload: ByteArray,
    val checksum: Int? = null,
)
