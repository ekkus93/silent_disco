package com.ekkus.silentdisco.core.audio

import android.os.SystemClock
import com.ekkus.silentdisco.core.logging.AppLogger
import com.ekkus.silentdisco.core.uniffi.FfiAudioOutputHandle

private const val RENDER_RING_CAPACITY_FRAMES: UInt = 48_000u
private const val RENDER_RING_TARGET_FILL_FRAMES: UInt = 19_200u
private const val PCM16_FULL_SCALE = 32_768.0f
private const val OBOE_ADAPTER_STATUS_OK = 0
private const val RING_FULL_RETRY_DELAY_MS = 2L
private const val MAX_STALL_RETRIES = 500

/**
 * Legacy adapter retained for focused PCM/ring/Oboe regression tests.
 *
 * Production listener and Android host-monitor playback use
 * `FfiListenerPlaybackHandle`, so Kotlin no longer decides what plays or
 * converts scheduled PCM into the render ring. This class remains useful as
 * an isolated low-level test surface for the native Oboe bridge.
 */
class OboePlaybackEngine : PlaybackEngine {
    private var handle: FfiAudioOutputHandle? = null
    private var volume: Float = 1.0f
    private var writeCount: Long = 0
    private var channelCount: Int = AudioFormatSpec().channelCount
    private val logger = AppLogger("OboePlaybackEngine")
    private var stallEventCount: Long = 0
    private var stallTotalMs: Long = 0

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
        val samples = pcm16LeToFloat(frame.packet.payload, volume)
        val framesWritten = pushFully(samples, stallLabel = "seq=${frame.packet.sequenceNumber}")
        writeCount += 1
        return framesWritten.toLong()
    }

    /**
     * Pushes every frame of [samples] into the ring, retrying the unwritten
     * remainder until the ring accepts all of it.
     *
     * [FfiAudioOutputHandleInterface.pushFrames] returns only the count
     * *actually* written -- a temporarily full ring can accept fewer frames
     * than requested. An earlier implementation discarded that count
     * entirely and always reported the whole frame as written, silently
     * dropping whatever the ring didn't accept: audible as a dropped or
     * corrupted note, not a clean gap. Retrying (with a short backoff so a
     * full ring gets a chance to drain) makes this call genuinely block
     * until the frame is fully queued, which is the correct backpressure
     * signal for the caller's pacing loop.
     *
     * Only a genuine retry (the ring was observed full at least once)
     * counts as a stall. Timing the whole call instead -- as an earlier
     * version of this instrumentation did -- mostly measured the fixed
     * per-call overhead of boxing samples into a `List<Float>` for the
     * JNI/UniFFI crossing (~5ms on roughly half of all calls even with
     * zero retries): a real cost, but not backpressure, and not what the
     * stall diagnostic is for.
     */
    private fun pushFully(samples: FloatArray, stallLabel: String): Int {
        val active = handle ?: error("Playback engine is not started")
        val totalFrames = samples.size / channelCount
        var framesWritten = 0
        var stallRetries = 0
        var totalStallRetries = 0
        val startedAtMs = SystemClock.elapsedRealtime()
        while (framesWritten < totalFrames) {
            val remaining = samples.copyOfRange(framesWritten * channelCount, samples.size)
            val written = active.pushFrames(remaining.toList())
            if (written == 0u) {
                stallRetries += 1
                totalStallRetries += 1
                if (stallRetries > MAX_STALL_RETRIES) {
                    error("Render ring did not drain after $MAX_STALL_RETRIES retries")
                }
                Thread.sleep(RING_FULL_RETRY_DELAY_MS)
                continue
            }
            stallRetries = 0
            framesWritten += written.toInt()
        }
        if (totalStallRetries > 0) {
            val elapsedMs = SystemClock.elapsedRealtime() - startedAtMs
            stallEventCount += 1
            stallTotalMs += elapsedMs
            logger.w(
                "oboe.write_stall",
                "$stallLabel elapsedMs=$elapsedMs retries=$totalStallRetries " +
                    "stallEvents=$stallEventCount stallTotalMs=$stallTotalMs",
            )
        }
        return framesWritten
    }

    override fun playbackPositionMs(frame: PlaybackFrame): Long = frame.localDeadlineMs

    override fun stop() {
        // Only close the native stream this engine actually opened. The
        // adapter is process-global and the listener runtime may own it, so
        // an unconditional close here would silence a stream this engine
        // never started.
        val opened = handle
        handle = null
        if (opened != null) {
            OboeBridge.nativeOboeClose()
            opened.release()
        }
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

    /** Cumulative count and total wall-clock time of [write] calls that stalled waiting for the ring to drain. */
    fun stallSummary(): String = "stallEvents=$stallEventCount stallTotalMs=$stallTotalMs"
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
