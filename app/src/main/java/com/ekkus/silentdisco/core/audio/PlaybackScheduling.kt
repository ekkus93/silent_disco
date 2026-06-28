package com.ekkus.silentdisco.core.audio

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.os.SystemClock
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
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
    val invalidPacketCount: Int = 0,
    val concealedPacketCount: Int = 0,
    val lastPlayedSequence: Long? = null,
    val bufferDepthMs: Long = 0,
    val shouldResync: Boolean = false,
)

data class PlaybackFrame(
    val packet: AudioPacket,
    val localDeadlineMs: Long,
    val concealed: Boolean = false,
)

class ListenerPlaybackScheduler(
    private val mapper: HostTimeMapper,
    private val thresholds: PlaybackThresholds = PlaybackThresholds(),
    private val buffer: AudioPacketBuffer = AudioPacketBuffer(startupTargetMs = thresholds.startupBufferMs),
    private val expectedSessionId: SessionId? = null,
    private val expectedStreamId: StreamId? = null,
    private val nowProvider: () -> Long = { SystemClock.elapsedRealtime() },
) {
    private var packetLossCount = 0
    private var lateDropCount = 0
    private var underrunCount = 0
    private var invalidPacketCount = 0
    private var concealedPacketCount = 0
    private var lastDeliveredSequence: Long? = null
    private var lastDeliveredPacket: AudioPacket? = null
    private var lastPlaybackErrorMs: Long = 0

    fun submit(packet: AudioPacket): PlaybackTelemetry {
        if ((expectedSessionId != null && packet.sessionId != expectedSessionId) ||
            (expectedStreamId != null && packet.streamId != expectedStreamId)
        ) {
            invalidPacketCount += 1
            return snapshot(shouldResync = true)
        }
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
        concealedFrame(nowLocalTimeMs)?.let { concealed ->
            updateLastDelivered(nowLocalTimeMs, concealed)
            concealedPacketCount += 1
            return concealed
        }
        val ready = buffer.popReady(nowLocalTimeMs) ?: run {
            if (buffer.isReady()) {
                underrunCount += 1
                concealedUnderflowFrame(nowLocalTimeMs)?.let { concealed ->
                    updateLastDelivered(nowLocalTimeMs, concealed)
                    concealedPacketCount += 1
                    return concealed
                }
            }
            return null
        }
        val frame = PlaybackFrame(
            packet = ready.packet,
            localDeadlineMs = ready.scheduledLocalTimeMs,
        )
        updateLastDelivered(nowLocalTimeMs, frame)
        return frame
    }

    private fun updateLastDelivered(nowLocalTimeMs: Long, frame: PlaybackFrame) {
        lastDeliveredSequence = frame.packet.sequenceNumber
        lastDeliveredPacket = frame.packet
        lastPlaybackErrorMs = nowLocalTimeMs - frame.localDeadlineMs
    }

    private fun concealedFrame(nowLocalTimeMs: Long): PlaybackFrame? {
        val previous = lastDeliveredPacket ?: return null
        val nextReady = buffer.peekFirst() ?: return null
        val expectedSequence = previous.sequenceNumber + 1
        if (nextReady.packet.sequenceNumber <= expectedSequence || nextReady.scheduledLocalTimeMs > nowLocalTimeMs) {
            return null
        }
        return synthesizeConcealment(previous = previous, sequenceNumber = expectedSequence)
    }

    private fun concealedUnderflowFrame(nowLocalTimeMs: Long): PlaybackFrame? {
        val previous = lastDeliveredPacket ?: return null
        val expectedSequence = previous.sequenceNumber + 1
        val concealment = synthesizeConcealment(previous = previous, sequenceNumber = expectedSequence)
        return if (concealment.localDeadlineMs <= nowLocalTimeMs + thresholds.softCorrectionThresholdMs) {
            concealment
        } else {
            null
        }
    }

    private fun synthesizeConcealment(previous: AudioPacket, sequenceNumber: Long): PlaybackFrame {
        val packetDurationMs = (previous.samplesPerPacket * 1_000L / previous.sampleRate).coerceAtLeast(1L)
        val concealedPacket = previous.copy(
            sequenceNumber = sequenceNumber,
            firstSampleIndex = previous.firstSampleIndex + previous.samplesPerPacket,
            hostPresentationTimeMs = previous.hostPresentationTimeMs + packetDurationMs,
            payload = SilenceFiller.pcm16Le(
                frameCount = previous.samplesPerPacket,
                format = AudioFormatSpec(
                    sampleRate = previous.sampleRate,
                    channelCount = previous.channelCount,
                ),
            ),
        )
        return PlaybackFrame(
            packet = concealedPacket,
            localDeadlineMs = mapper.hostToLocal(concealedPacket.hostPresentationTimeMs),
            concealed = true,
        )
    }

    fun snapshot(shouldResync: Boolean = false): PlaybackTelemetry = PlaybackTelemetry(
        packetLossCount = packetLossCount + buffer.missingSequenceCount(),
        lateDropCount = lateDropCount,
        underrunCount = underrunCount,
        invalidPacketCount = invalidPacketCount,
        concealedPacketCount = concealedPacketCount,
        lastPlayedSequence = lastDeliveredSequence,
        bufferDepthMs = buffer.depthMs(),
        shouldResync = shouldResync || kotlin.math.abs(lastPlaybackErrorMs) > thresholds.hardResyncThresholdMs,
    )
}

interface PlaybackEngine {
    fun start(format: AudioFormatSpec = AudioFormatSpec()): String
    fun write(frame: PlaybackFrame): Long
    fun setVolume(value: Float)
    fun playbackPositionMs(frame: PlaybackFrame): Long
    fun stop()
}

class AudioTrackPlaybackEngine : PlaybackEngine {
    private var audioTrack: AudioTrack? = null
    private var sampleRate: Int = 48_000
    private var writeCount: Long = 0
    private var volume: Float = 1.0f

    override fun start(format: AudioFormatSpec): String {
        sampleRate = format.sampleRate
        if (audioTrack == null) {
            val channelMask = if (format.channelCount == 1) {
                AudioFormat.CHANNEL_OUT_MONO
            } else {
                AudioFormat.CHANNEL_OUT_STEREO
            }
            val minBufferSize = AudioTrack.getMinBufferSize(
                format.sampleRate,
                channelMask,
                AudioFormat.ENCODING_PCM_16BIT,
            ).coerceAtLeast(format.sampleRate / 5 * format.bytesPerFrame)
            audioTrack = AudioTrack.Builder()
                .setAudioAttributes(
                    AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_MEDIA)
                        .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                        .build(),
                )
                .setAudioFormat(
                    AudioFormat.Builder()
                        .setSampleRate(format.sampleRate)
                        .setChannelMask(channelMask)
                        .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                        .build(),
                )
                .setTransferMode(AudioTrack.MODE_STREAM)
                .setBufferSizeInBytes(minBufferSize)
                .build()
                .also {
                    it.setVolume(volume)
                    it.play()
                }
        }
        return "Android AudioTrack"
    }

    override fun write(frame: PlaybackFrame): Long {
        val track = audioTrack ?: error("Playback engine is not started")
        val written = track.write(
            frame.packet.payload,
            0,
            frame.packet.payload.size,
            AudioTrack.WRITE_NON_BLOCKING,
        )
        if (written <= 0) {
            error("AudioTrack write failed with result=$written")
        }
        writeCount += 1
        return written.toLong()
    }

    override fun playbackPositionMs(frame: PlaybackFrame): Long {
        val headPosition = audioTrack?.playbackHeadPosition?.toLong() ?: return frame.localDeadlineMs
        return (headPosition * 1_000L) / sampleRate
    }

    override fun stop() {
        audioTrack?.pause()
        audioTrack?.flush()
        audioTrack?.release()
        audioTrack = null
    }

    override fun setVolume(value: Float) {
        volume = value.coerceIn(0f, 1f)
        audioTrack?.setVolume(volume)
    }

    fun statusSummary(): String = "writes=$writeCount, ${OboeBridge.statusSummary()}"
}

@Deprecated(
    message = "Use AudioTrackPlaybackEngine. Playback output is Android AudioTrack-backed; the native Oboe bridge is diagnostics-only.",
    replaceWith = ReplaceWith("AudioTrackPlaybackEngine"),
)
typealias OboePlaybackEngine = AudioTrackPlaybackEngine
