package com.ekkus.silentdisco.core.audio

import com.ekkus.silentdisco.core.uniffi.FfiAudioOutputHandle

private const val RENDER_RING_CAPACITY_FRAMES: UInt = 48_000u
private const val RENDER_RING_TARGET_FILL_FRAMES: UInt = 19_200u
private const val PCM16_FULL_SCALE = 32_768.0f
private const val OBOE_ADAPTER_STATUS_OK = 0
private const val RING_FULL_RETRY_DELAY_MS = 2L
private const val MAX_STALL_RETRIES = 500

/**
 * Production listener/host-monitor output: Oboe (native, via [OboeBridge])
 * consuming the shared Rust render ring. Kotlin still decides what plays and
 * when, exactly as before; this engine only converts each already-scheduled
 * frame's PCM16LE payload to interleaved float32 and pushes it into the
 * ring. It never writes to any audio hardware API itself — the native Oboe
 * callback owns that, reading from the ring through the narrow real-time C
 * ABI (`include/silent_disco_audio.h`).
 */
class OboePlaybackEngine : PlaybackEngine {
    private var handle: FfiAudioOutputHandle? = null
    private var volume: Float = 1.0f
    private var writeCount: Long = 0
    private var channelCount: Int = AudioFormatSpec().channelCount

    override fun start(format: AudioFormatSpec): String {
        channelCount = format.channelCount
        handle?.let { return "Oboe" }
        val opened = FfiAudioOutputHandle.open(RENDER_RING_CAPACITY_FRAMES, RENDER_RING_TARGET_FILL_FRAMES)
        val token = opened.engineToken()
        val status = OboeBridge.nativeOboeOpen(token)
        if (status != OBOE_ADAPTER_STATUS_OK) {
            opened.release()
            error("Oboe stream failed to open (status=$status)")
        }
        handle = opened
        return "Oboe"
    }

    /**
     * Pushes one frame's samples into the render ring, retrying the
     * unwritten remainder until the ring accepts all of it.
     *
     * [FfiAudioOutputHandleInterface.pushFrames] returns only the count
     * *actually* written -- a temporarily full ring can accept fewer frames
     * than requested. The previous implementation discarded that count
     * entirely and always reported the whole frame as written, silently
     * dropping whatever the ring didn't accept: audible as a dropped or
     * corrupted note, not a clean gap. Retrying (with a short backoff so a
     * full ring gets a chance to drain) makes this call genuinely block
     * until the frame is fully queued, which is the correct backpressure
     * signal for the caller's real-time pacing loop.
     */
    override fun write(frame: PlaybackFrame): Long {
        val active = handle ?: error("Playback engine is not started")
        val samples = pcm16LeToFloat(frame.packet.payload, volume)
        val totalFrames = samples.size / channelCount
        var framesWritten = 0
        var stallRetries = 0
        while (framesWritten < totalFrames) {
            val remaining = samples.copyOfRange(framesWritten * channelCount, samples.size)
            val written = active.pushFrames(remaining.toList())
            if (written == 0u) {
                stallRetries += 1
                if (stallRetries > MAX_STALL_RETRIES) {
                    error("Render ring did not drain after $MAX_STALL_RETRIES retries")
                }
                Thread.sleep(RING_FULL_RETRY_DELAY_MS)
                continue
            }
            stallRetries = 0
            framesWritten += written.toInt()
        }
        writeCount += 1
        return framesWritten.toLong()
    }

    override fun playbackPositionMs(frame: PlaybackFrame): Long = frame.localDeadlineMs

    override fun stop() {
        OboeBridge.nativeOboeClose()
        handle?.release()
        handle = null
    }

    override fun setVolume(value: Float) {
        volume = value.coerceIn(0f, 1f)
    }

    /** Last fatal status the real-time callback observed since the previous check, or 0 if none. */
    fun takeFatalStatus(): Int = OboeBridge.nativeOboeTakeFatalStatus()

    /** True if the stream disconnected (e.g. a route change) since the previous check. */
    fun takeDisconnected(): Boolean = OboeBridge.nativeOboeTakeDisconnected()

    fun statusSummary(): String = "writes=$writeCount, backend=${OboeBridge.backendSummary()}, " +
        "sampleRate=${OboeBridge.nativeOboeActualSampleRate()}, " +
        "channels=${OboeBridge.nativeOboeActualChannelCount()}"
}

internal fun pcm16LeToFloat(payload: ByteArray, volume: Float): FloatArray {
    val sampleCount = payload.size / 2
    val samples = FloatArray(sampleCount)
    var byteIndex = 0
    for (index in 0 until sampleCount) {
        val low = payload[byteIndex].toInt() and 0xFF
        val high = payload[byteIndex + 1].toInt() and 0xFF
        val raw = ((high shl 8) or low).toShort()
        samples[index] = (raw / PCM16_FULL_SCALE) * volume
        byteIndex += 2
    }
    return samples
}
