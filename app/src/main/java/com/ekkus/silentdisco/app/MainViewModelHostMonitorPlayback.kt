package com.ekkus.silentdisco.app

import android.os.SystemClock
import com.ekkus.silentdisco.core.audio.AudioFormatSpec
import com.ekkus.silentdisco.core.audio.OboeBridge
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.uniffi.FfiAudioPacket
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackConfig
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle

private const val HOST_MONITOR_OBOE_STATUS_OK = 0
private const val HOST_MONITOR_RING_CAPACITY_FRAMES: UInt = 48_000u
private const val HOST_MONITOR_RING_TARGET_FILL_FRAMES: UInt = 19_200u
private const val HOST_MONITOR_WRITE_LEAD_MS: ULong = 400uL
private const val HOST_MONITOR_MAX_PREFILL_MS: ULong = 800uL

/**
 * Opens the Android host's local monitor on the same Rust scheduler/pump/ring
 * runtime used by listeners. Network broadcast pacing remains owned by the
 * host stream loop; this runtime only decides what the host itself hears.
 */
internal fun MainViewModel.startHostMonitorPlayback(
    sessionId: SessionId,
    streamId: StreamId,
    format: AudioFormatSpec,
    firstPacket: AudioPacket,
): String {
    stopHostMonitorPlayback()
    require(firstPacket.streamId == streamId) { "host monitor packet stream does not match active stream" }
    require(firstPacket.sessionId == sessionId) { "host monitor packet session does not match active session" }

    val runtime = FfiListenerPlaybackHandle.open(
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
            volume = uiState.value.localVolume,
        ),
    )

    try {
        // Host packet timestamps use elapsedRealtime(), while the Rust pump
        // clock starts at zero when this runtime opens. Rust owns the mapping.
        runtime.lockSameProcessHostClock(SystemClock.elapsedRealtime().toULong())
        val oboeStatus = OboeBridge.nativeOboeOpen(runtime.engineToken())
        check(oboeStatus == HOST_MONITOR_OBOE_STATUS_OK) {
            "Oboe stream failed to open for host monitor (status=$oboeStatus)"
        }
        hostMonitorPlayback = runtime
        return "Rust/Oboe host monitor"
    } catch (error: Throwable) {
        runCatching { runtime.stop() }.exceptionOrNull()?.let(error::addSuppressed)
        runCatching { OboeBridge.nativeOboeClose() }.exceptionOrNull()?.let(error::addSuppressed)
        runCatching { runtime.close() }.exceptionOrNull()?.let(error::addSuppressed)
        throw error
    }
}

/** Submits one locally generated host packet into the Rust-owned monitor runtime. */
internal fun MainViewModel.submitHostMonitorPacket(packet: AudioPacket) {
    val runtime = hostMonitorPlayback ?: error("Host monitor playback is not running")
    runtime.submitPacket(
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
internal fun MainViewModel.reanchorHostMonitorPlayback(hostStartTimeMs: Long) {
    require(hostStartTimeMs >= 0L) { "host monitor anchor must be non-negative" }
    hostMonitorPlayback?.reanchorPresentationTime(hostStartTimeMs.toULong())
}

/**
 * Drains/stops the Rust monitor before closing Oboe, preserving the first
 * failure while still attempting every cleanup step. Idempotent.
 */
internal fun MainViewModel.stopHostMonitorPlayback() {
    val runtime = swapHostMonitorPlayback(null) ?: return
    var firstFailure: Throwable? = null
    fun capture(block: () -> Unit) {
        try {
            block()
        } catch (error: Throwable) {
            val first = firstFailure
            if (first == null) firstFailure = error else first.addSuppressed(error)
        }
    }
    capture { runtime.stop() }
    capture { OboeBridge.nativeOboeClose() }
    capture { runtime.close() }
    firstFailure?.let { throw it }
}
