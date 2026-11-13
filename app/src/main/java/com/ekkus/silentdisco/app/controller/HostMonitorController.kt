package com.ekkus.silentdisco.app.controller

import android.os.SystemClock
import com.ekkus.silentdisco.core.audio.AudioFormatSpec
import com.ekkus.silentdisco.core.audio.OboeBridge
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.uniffi.FfiAudioPacket
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackConfig
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.CoroutineScope

private const val HOST_MONITOR_OBOE_STATUS_OK = 0
private const val HOST_MONITOR_RING_CAPACITY_FRAMES: UInt = 48_000u
private const val HOST_MONITOR_RING_TARGET_FILL_FRAMES: UInt = 19_200u
private const val HOST_MONITOR_WRITE_LEAD_MS: ULong = 400uL
private const val HOST_MONITOR_MAX_PREFILL_MS: ULong = 800uL

/**
 * The Android host's local monitor: what the host phone itself hears (God-file
 * refactor spec §4). It runs on the same Rust scheduler/pump/ring runtime as
 * listeners. Network broadcast pacing stays with the host stream loop.
 *
 * The native seams are injectable because JVM tests cannot load the native
 * libraries.
 */
internal class HostMonitorController(
    parentScope: CoroutineScope,
    private val closeOutput: () -> Unit = OboeBridge::nativeOboeClose,
    private val openOutput: (engineToken: Long) -> Int = OboeBridge::nativeOboeOpen,
    private val openRuntime: (FfiListenerPlaybackConfig) -> FfiListenerPlaybackHandle =
        { FfiListenerPlaybackHandle.open(it) },
    private val elapsedRealtimeMs: () -> Long = SystemClock::elapsedRealtime,
) : ViewModelController(parentScope) {
    private val runtimeRef = AtomicReference<FfiListenerPlaybackHandle?>(null)

    /** The running monitor runtime, if any. */
    var runtime: FfiListenerPlaybackHandle?
        get() = runtimeRef.get()
        set(value) = runtimeRef.set(value)

    /** Replaces the runtime and returns the previous one; nothing is stopped. */
    fun swap(value: FfiListenerPlaybackHandle?): FfiListenerPlaybackHandle? = runtimeRef.getAndSet(value)

    /** Opens the monitor for a new stream, stopping any running one first. */
    fun start(
        sessionId: SessionId,
        streamId: StreamId,
        format: AudioFormatSpec,
        firstPacket: AudioPacket,
        volume: Float,
    ) {
        stop()
        require(firstPacket.streamId == streamId) { "host monitor packet stream does not match active stream" }
        require(firstPacket.sessionId == sessionId) { "host monitor packet session does not match active session" }

        val opened = openRuntime(
            FfiListenerPlaybackConfig(
                sessionId = sessionId.value,
                streamId = streamId.value,
                sampleRate = format.sampleRate.toUInt(),
                hostStartTimeMs = firstPacket.hostPresentationTimeMs.toULong(),
                samplesPerPacket = firstPacket.samplesPerPacket.toUInt(),
                channels = format.channelCount.toUShort(),
                // Local decoded audio has no network jitter to accumulate. The
                // presentation deadline + render-ring write lead provide pacing.
                startupBufferTargetMs = 0uL,
                rebufferTargetMs = 0uL,
                ringCapacityFrames = HOST_MONITOR_RING_CAPACITY_FRAMES,
                ringTargetFillFrames = HOST_MONITOR_RING_TARGET_FILL_FRAMES,
                writeLeadMs = HOST_MONITOR_WRITE_LEAD_MS,
                maxPrefillMs = HOST_MONITOR_MAX_PREFILL_MS,
                volume = volume,
            ),
        )

        try {
            // Host packet timestamps use elapsedRealtime(), while the Rust pump
            // clock starts at zero when this runtime opens. Rust owns the mapping.
            opened.lockSameProcessHostClock(elapsedRealtimeMs().toULong())
            val oboeStatus = openOutput(opened.engineToken())
            check(oboeStatus == HOST_MONITOR_OBOE_STATUS_OK) {
                "Oboe stream failed to open for host monitor (status=$oboeStatus)"
            }
            runtime = opened
        } catch (error: Throwable) {
            runCatching { opened.stop() }.exceptionOrNull()?.let(error::addSuppressed)
            runCatching { closeOutput() }.exceptionOrNull()?.let(error::addSuppressed)
            runCatching { opened.close() }.exceptionOrNull()?.let(error::addSuppressed)
            throw error
        }
    }

    /** Submits one locally generated host packet into the monitor runtime. */
    fun submit(packet: AudioPacket) {
        val running = runtime ?: error("Host monitor playback is not running")
        running.submitPacket(
            FfiAudioPacket(
                sequence = packet.sequenceNumber.toULong(),
                sampleRate = packet.sampleRate.toUInt(),
                channels = packet.channelCount.toUShort(),
                samplesPerPacket = packet.samplesPerPacket.toUInt(),
                firstSampleIndex = packet.firstSampleIndex.toULong(),
                hostPresentationTimeMs = packet.hostPresentationTimeMs.toULong(),
                payload = packet.payload,
            ),
            packet.sessionId.value,
            packet.streamId.value,
        )
    }

    /** Re-anchors the live monitor after the host shifts packet time across a pause. */
    fun reanchor(hostStartTimeMs: Long) {
        require(hostStartTimeMs >= 0L) { "host monitor anchor must be non-negative" }
        runtime?.reanchorPresentationTime(hostStartTimeMs.toULong())
    }

    fun setVolume(volume: Float) {
        runtime?.setVolume(volume)
    }

    /**
     * Drains/stops the Rust monitor before closing Oboe, preserving the first
     * failure while still attempting every cleanup step. Idempotent.
     */
    fun stop() {
        val running = swap(null) ?: return
        var firstFailure: Throwable? = null
        fun capture(block: () -> Unit) {
            try {
                block()
            } catch (error: Throwable) {
                val first = firstFailure
                if (first == null) firstFailure = error else first.addSuppressed(error)
            }
        }
        capture { running.stop() }
        capture { closeOutput() }
        capture { running.close() }
        firstFailure?.let { throw it }
    }

    /** `MainViewModel.onCleared` stops the monitor at its original point; this covers the rest. */
    override fun release() = stop()
}
