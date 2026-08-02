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

/**
 * Length of the linear amplitude ramp applied at every concealment boundary.
 * A lost packet replaced by instant silence produces two hard waveform
 * discontinuities -- one where real audio cuts to zero, one where it snaps
 * back -- each audible as a click or pop. Ramping over a few milliseconds
 * keeps each gap inaudible as a *click* (it remains a brief dip) without
 * being long enough to smear real content.
 */
private const val CONCEALMENT_RAMP_MS = 5

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
    private var highestSubmittedSequence: Long? = null
    private var fadeInNextRealFrame = true

    fun submit(packet: AudioPacket): PlaybackTelemetry {
        if ((expectedSessionId != null && packet.sessionId != expectedSessionId) ||
            (expectedStreamId != null && packet.streamId != expectedStreamId)
        ) {
            invalidPacketCount += 1
            return snapshot(shouldResync = true)
        }
        // Loss is arrival continuity: a forward jump past the highest sequence
        // seen so far counts each skipped packet once; a later out-of-order
        // arrival backfills one previously-counted hole. Comparing against the
        // *playback* head instead -- as this used to -- made every in-flight
        // send-ahead packet (routinely ~1s of them) count as "lost" on every
        // submit, compounding into a six-figure counter for ~1% real loss and
        // driving a per-packet warning-log storm on the reception path.
        val expectedNext = highestSubmittedSequence?.plus(1)
        if (expectedNext != null) {
            if (packet.sequenceNumber > expectedNext) {
                packetLossCount += (packet.sequenceNumber - expectedNext).toInt()
            } else if (packet.sequenceNumber < expectedNext) {
                packetLossCount = (packetLossCount - 1).coerceAtLeast(0)
            }
        }
        if (packet.sequenceNumber > (highestSubmittedSequence ?: Long.MIN_VALUE)) {
            highestSubmittedSequence = packet.sequenceNumber
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

    /**
     * Drains and returns any buffered-but-not-yet-due frames in sequence
     * order. Call when a stream is stopping so its tail -- real audio the
     * network already delivered, just not yet at its scheduled deadline --
     * gets played instead of silently discarded. The send-ahead horizon
     * means up to roughly a second of legitimate content can be sitting
     * here at any moment, not only right at stream end.
     *
     * Drained frames play back-to-back, so a sequence hole (a packet lost
     * near stream end that no concealment ever bridged) would butt two
     * non-adjacent waveforms directly together -- a hard click with no
     * silence gap for analysis to even see. Every hole edge is ramped:
     * fade-out into the hole, fade-in coming out of it, plus a fade-out on
     * the final frame's tail so the stream ends at zero instead of cutting
     * mid-waveform when the engine stops.
     */
    fun drainRemaining(): List<PlaybackFrame> {
        val drained = buffer.drainAll()
        if (drained.isEmpty()) return emptyList()
        val frames = drained.map { PlaybackFrame(packet = it.packet, localDeadlineMs = it.scheduledLocalTimeMs) }
            .toMutableList()
        var previousSequence = lastDeliveredSequence
        for (index in frames.indices) {
            val frame = frames[index]
            val expected = previousSequence?.plus(1)
            if (expected != null && frame.packet.sequenceNumber > expected) {
                if (index > 0) {
                    frames[index - 1] = frames[index - 1].withFadedTail()
                }
                frames[index] = frame.withFadedHead()
            }
            previousSequence = frame.packet.sequenceNumber
        }
        frames[frames.lastIndex] = frames[frames.lastIndex].withFadedTail()
        return frames
    }

    private fun PlaybackFrame.withFadedHead(): PlaybackFrame = copy(
        packet = packet.copy(
            payload = pcm16LeFadeIn(packet.payload, packet.channelCount, rampFrames(packet.sampleRate)),
            checksum = null,
        ),
    )

    private fun PlaybackFrame.withFadedTail(): PlaybackFrame = copy(
        packet = packet.copy(
            payload = pcm16LeFadeOutTail(packet.payload, packet.channelCount, rampFrames(packet.sampleRate)),
            checksum = null,
        ),
    )

    fun poll(nowLocalTimeMs: Long = SystemClock.elapsedRealtime()): PlaybackFrame? {
        concealedFrame(nowLocalTimeMs)?.let { concealed ->
            updateLastDelivered(nowLocalTimeMs, concealed)
            concealedPacketCount += 1
            fadeInNextRealFrame = true
            return concealed
        }
        val ready = buffer.popReady(nowLocalTimeMs) ?: run {
            if (buffer.isReady()) {
                underrunCount += 1
                concealedUnderflowFrame(nowLocalTimeMs)?.let { concealed ->
                    updateLastDelivered(nowLocalTimeMs, concealed)
                    concealedPacketCount += 1
                    fadeInNextRealFrame = true
                    return concealed
                }
            }
            return null
        }
        // The first real frame after any concealment (and the very first
        // frame of a stream, which routinely starts mid-note after the
        // startup backlog flush) resumes from silence: ramp it in so the
        // resume edge is a dip, not a click.
        val deliveredPacket = if (fadeInNextRealFrame) {
            fadeInNextRealFrame = false
            ready.packet.copy(
                payload = pcm16LeFadeIn(
                    payload = ready.packet.payload,
                    channelCount = ready.packet.channelCount,
                    rampFrames = rampFrames(ready.packet.sampleRate),
                ),
                checksum = null,
            )
        } else {
            ready.packet
        }
        val frame = PlaybackFrame(
            packet = deliveredPacket,
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
        // If the expected packet is already buffered and simply not due yet,
        // there is nothing to conceal: synthesizing silence now would fill
        // the slot whose real audio is about to play on time, and the real
        // packet would then play as well -- a duplicated slot that dilates
        // the stream by one packet duration per occurrence.
        val head = buffer.peekFirst()
        if (head != null && head.packet.sequenceNumber <= expectedSequence) return null
        val concealment = synthesizeConcealment(previous = previous, sequenceNumber = expectedSequence)
        return if (concealment.localDeadlineMs <= nowLocalTimeMs + thresholds.softCorrectionThresholdMs) {
            concealment
        } else {
            null
        }
    }

    private fun synthesizeConcealment(previous: AudioPacket, sequenceNumber: Long): PlaybackFrame {
        val packetDurationMs = (previous.samplesPerPacket * 1_000L / previous.sampleRate).coerceAtLeast(1L)
        // Ramp the previous packet's final sample values down to zero instead
        // of cutting straight to silence: the gap becomes a fade, not a click.
        // A concealment following another concealment ramps from an
        // already-zero tail, so multi-packet holes stay pure silence after
        // the first fade with no special casing.
        val concealedPacket = previous.copy(
            sequenceNumber = sequenceNumber,
            firstSampleIndex = previous.firstSampleIndex + previous.samplesPerPacket,
            hostPresentationTimeMs = previous.hostPresentationTimeMs + packetDurationMs,
            payload = pcm16LeRampToSilence(
                previousPayload = previous.payload,
                frameCount = previous.samplesPerPacket,
                channelCount = previous.channelCount,
                rampFrames = rampFrames(previous.sampleRate),
            ),
            checksum = null,
        )
        return PlaybackFrame(
            packet = concealedPacket,
            localDeadlineMs = mapper.hostToLocal(concealedPacket.hostPresentationTimeMs),
            concealed = true,
        )
    }

    private fun rampFrames(sampleRate: Int): Int = (sampleRate * CONCEALMENT_RAMP_MS / 1_000).coerceAtLeast(1)

    fun snapshot(shouldResync: Boolean = false): PlaybackTelemetry = PlaybackTelemetry(
        packetLossCount = packetLossCount,
        lateDropCount = lateDropCount,
        underrunCount = underrunCount,
        invalidPacketCount = invalidPacketCount,
        concealedPacketCount = concealedPacketCount,
        lastPlayedSequence = lastDeliveredSequence,
        bufferDepthMs = buffer.depthMs(),
        shouldResync = shouldResync || kotlin.math.abs(lastPlaybackErrorMs) > thresholds.hardResyncThresholdMs,
    )
}

/**
 * Builds a PCM16LE payload of [frameCount] frames that linearly ramps each
 * channel from [previousPayload]'s final frame values down to zero over
 * [rampFrames] frames, then stays silent. An empty or sub-frame previous
 * payload yields pure silence.
 */
internal fun pcm16LeRampToSilence(
    previousPayload: ByteArray,
    frameCount: Int,
    channelCount: Int,
    rampFrames: Int,
): ByteArray {
    val bytesPerFrame = channelCount * 2
    val output = ByteArray(frameCount * bytesPerFrame)
    val previousFrames = previousPayload.size / bytesPerFrame
    if (previousFrames == 0) return output
    val lastFrameOffset = (previousFrames - 1) * bytesPerFrame
    val effectiveRamp = rampFrames.coerceIn(1, frameCount)
    for (channel in 0 until channelCount) {
        val byteIndex = lastFrameOffset + channel * 2
        val low = previousPayload[byteIndex].toInt() and 0xFF
        val high = previousPayload[byteIndex + 1].toInt() and 0xFF
        val lastValue = ((high shl 8) or low).toShort().toInt()
        for (frame in 0 until effectiveRamp) {
            val scaled = lastValue * (effectiveRamp - 1 - frame) / effectiveRamp
            val outIndex = frame * bytesPerFrame + channel * 2
            output[outIndex] = (scaled and 0xFF).toByte()
            output[outIndex + 1] = ((scaled shr 8) and 0xFF).toByte()
        }
    }
    return output
}

/**
 * Returns a copy of [payload] whose first [rampFrames] frames linearly fade
 * in from zero, leaving the rest untouched. Used when real audio resumes
 * after a concealment gap (or starts mid-note), so the resume edge is a
 * short dip rather than an instantaneous step.
 */
internal fun pcm16LeFadeIn(payload: ByteArray, channelCount: Int, rampFrames: Int): ByteArray {
    val bytesPerFrame = channelCount * 2
    val totalFrames = payload.size / bytesPerFrame
    if (totalFrames == 0) return payload.copyOf()
    val output = payload.copyOf()
    val effectiveRamp = rampFrames.coerceIn(1, totalFrames)
    for (frame in 0 until effectiveRamp) {
        for (channel in 0 until channelCount) {
            val byteIndex = frame * bytesPerFrame + channel * 2
            val low = output[byteIndex].toInt() and 0xFF
            val high = output[byteIndex + 1].toInt() and 0xFF
            val value = ((high shl 8) or low).toShort().toInt()
            val scaled = value * frame / effectiveRamp
            output[byteIndex] = (scaled and 0xFF).toByte()
            output[byteIndex + 1] = ((scaled shr 8) and 0xFF).toByte()
        }
    }
    return output
}

/**
 * Returns a copy of [payload] whose final [rampFrames] frames linearly fade
 * out to zero, leaving the rest untouched. Used at drain-time hole edges and
 * on the final frame of a stream so playback never cuts mid-waveform.
 */
internal fun pcm16LeFadeOutTail(payload: ByteArray, channelCount: Int, rampFrames: Int): ByteArray {
    val bytesPerFrame = channelCount * 2
    val totalFrames = payload.size / bytesPerFrame
    if (totalFrames == 0) return payload.copyOf()
    val output = payload.copyOf()
    val effectiveRamp = rampFrames.coerceIn(1, totalFrames)
    val rampStart = totalFrames - effectiveRamp
    for (frame in rampStart until totalFrames) {
        val remaining = totalFrames - 1 - frame
        for (channel in 0 until channelCount) {
            val byteIndex = frame * bytesPerFrame + channel * 2
            val low = output[byteIndex].toInt() and 0xFF
            val high = output[byteIndex + 1].toInt() and 0xFF
            val value = ((high shl 8) or low).toShort().toInt()
            val scaled = value * remaining / effectiveRamp
            output[byteIndex] = (scaled and 0xFF).toByte()
            output[byteIndex + 1] = ((scaled shr 8) and 0xFF).toByte()
        }
    }
    return output
}

interface PlaybackEngine {
    fun start(format: AudioFormatSpec = AudioFormatSpec()): String
    fun write(frame: PlaybackFrame): Long
    fun setVolume(value: Float)
    fun playbackPositionMs(frame: PlaybackFrame): Long
    fun stop()
}

/**
 * Legacy Android `AudioTrack`-backed engine. No longer the production
 * output path — [OboePlaybackEngine] is. Retained only as a reference
 * implementation and for its existing JVM unit test coverage; not wired
 * into [com.ekkus.silentdisco.app.MainViewModel].
 */
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
