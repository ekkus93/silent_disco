package com.ekkus.silentdisco.core.audio

import android.os.SystemClock
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.sync.HostTimeMapper

data class PlaybackThresholds(
    val startupBufferMs: Long = 400,
    val softCorrectionThresholdMs: Long = 40,
    val hardResyncThresholdMs: Long = 120,
)

data class PlaybackTelemetry(
    val packetLossCount: Int = 0,
    val lateDropCount: Int = 0,
    val underrunCount: Int = 0,
    val lastPlayedSequence: Long? = null,
    val bufferDepthMs: Long = 0,
    val shouldResync: Boolean = false,
)

data class PlaybackFrame(
    val packet: AudioPacket,
    val localDeadlineMs: Long,
)

class ListenerPlaybackScheduler(
    private val mapper: HostTimeMapper,
    private val thresholds: PlaybackThresholds = PlaybackThresholds(),
    private val buffer: AudioPacketBuffer = AudioPacketBuffer(startupTargetMs = thresholds.startupBufferMs),
    private val nowProvider: () -> Long = { SystemClock.elapsedRealtime() },
) {
    private var packetLossCount = 0
    private var lateDropCount = 0
    private var underrunCount = 0
    private var lastDeliveredSequence: Long? = null
    private var lastPlaybackErrorMs: Long = 0

    fun submit(packet: AudioPacket): PlaybackTelemetry {
        if (lastDeliveredSequence != null && packet.sequenceNumber > lastDeliveredSequence!! + 1) {
            packetLossCount += (packet.sequenceNumber - lastDeliveredSequence!! - 1).toInt()
        }
        val deadline = mapper.hostToLocal(packet.hostPresentationTimeMs)
        val now = nowProvider()
        if (deadline < now - thresholds.softCorrectionThresholdMs) {
            lateDropCount += 1
            return snapshot(shouldResync = true)
        }
        buffer.insert(BufferedAudioPacket(packet, deadline))
        return snapshot()
    }

    fun canStart(): Boolean = buffer.isReady()

    fun poll(nowLocalTimeMs: Long = SystemClock.elapsedRealtime()): PlaybackFrame? {
        val ready = buffer.popReady(nowLocalTimeMs) ?: run {
            if (buffer.isReady()) {
                underrunCount += 1
            }
            return null
        }
        lastDeliveredSequence = ready.packet.sequenceNumber
        lastPlaybackErrorMs = nowLocalTimeMs - ready.scheduledLocalTimeMs
        return PlaybackFrame(
            packet = ready.packet,
            localDeadlineMs = ready.scheduledLocalTimeMs,
        )
    }

    fun snapshot(shouldResync: Boolean = false): PlaybackTelemetry = PlaybackTelemetry(
        packetLossCount = packetLossCount + buffer.missingSequenceCount(),
        lateDropCount = lateDropCount,
        underrunCount = underrunCount,
        lastPlayedSequence = lastDeliveredSequence,
        bufferDepthMs = buffer.depthMs(),
        shouldResync = shouldResync || kotlin.math.abs(lastPlaybackErrorMs) > thresholds.hardResyncThresholdMs,
    )
}

class OboePlaybackEngine {
    fun start(): String = OboeBridge.backendSummary()
    fun write(frame: PlaybackFrame): Long = frame.packet.payload.size.toLong()
    fun playbackPositionMs(frame: PlaybackFrame): Long = frame.localDeadlineMs
    fun stop() = Unit
}
