package com.ekkus.silentdisco.core.audio

import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import java.io.File
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/** Result of opt-in native callback capture. A dropped frame invalidates the WAV. */
data class CallbackAudioCaptureReport(
    val file: File,
    val wavFrames: Long,
    val capturedFrames: ULong,
    val lostFrames: ULong,
    val callbackCount: ULong,
    /** A cancelled or prematurely stopped writer cannot certify complete audio. */
    val tapFullyDrained: Boolean = false,
    val outputLifecycleAtStart: String,
    val outputLifecycleAtEnd: String,
    val outputCountersAtStart: OutputCallbackCounters? = null,
    val outputCountersAtEnd: OutputCallbackCounters? = null,
) {
    val complete: Boolean get() =
        tapFullyDrained && lostFrames == 0uL && wavFrames > 0 &&
            callbackCount > 0uL && wavFrames.toULong() == capturedFrames
    /** Unknown output state cannot count as a successful no-reopen check. */
    val newUnderrunCallbacks: Long?
        get() = outputCounterDelta(outputCountersAtStart?.underruns, outputCountersAtEnd?.underruns)
    val newSilenceFilledFrames: Long?
        get() = outputCounterDelta(outputCountersAtStart?.silenceFrames, outputCountersAtEnd?.silenceFrames)
    val noUnexpectedOutputReopen: Boolean?
        get() {
            val initial = parseOutputLifecycle(outputLifecycleAtStart) ?: return null
            val final = parseOutputLifecycle(outputLifecycleAtEnd) ?: return null
            // Historical opens alone do not prove an output is currently live:
            // opens=closes means every previously opened stream was closed.
            // An inactive stream cannot qualify callback continuity.
            // Exactly one live output must exist at each endpoint. Merely
            // observing unchanged cumulative opens/closes can conceal a
            // leaked second output or an impossible lifecycle snapshot.
            return initial.opens - initial.closes == 1 &&
                final.opens - final.closes == 1 &&
                initial.opens == final.opens &&
                initial.closes == final.closes &&
                // Rebinds are cumulative too: a reset is not valid evidence.
                final.rebinds >= initial.rebinds
        }
}

internal data class OutputLifecycle(val opens: Int, val rebinds: Int, val closes: Int)

/** Native output telemetry is only meaningful while the same Oboe stream remains open. */
data class OutputCallbackCounters(val underruns: Long, val silenceFrames: Long)

internal fun outputCounterDelta(before: Long?, after: Long?): Long? =
    if (before != null && after != null && before >= 0 && after >= before) after - before else null

internal fun currentOutputCallbackCounters(): OutputCallbackCounters? {
    if (!OboeBridge.isAvailable) return null
    return runCatching {
        if (!OboeBridge.nativeOboeIsOpen()) null else OutputCallbackCounters(
            underruns = OboeBridge.nativeOboeUnderrunCount(),
            silenceFrames = OboeBridge.nativeOboeSilenceFilledFrames(),
        )
    }.getOrNull()
}

internal fun parseOutputLifecycle(summary: String): OutputLifecycle? {
    val tokens = summary.split(' ')
    fun number(key: String): Int? =
        tokens.filter { it.startsWith("$key=") }.singleOrNull()?.substringAfter('=')?.toIntOrNull()
    val opens = number("opens")?.takeIf { it >= 0 } ?: return null
    val rebinds = number("rebinds")?.takeIf { it >= 0 } ?: return null
    val closes = number("closes")?.takeIf { it >= 0 } ?: return null
    return OutputLifecycle(opens, rebinds, closes)
}

/**
 * Opt-in listener capture for developer/device acceptance only. This runs on
 * Dispatchers.IO and consumes the real callback's preallocated Rust tap.
 * No files, allocations or mutex waits are introduced in the audio callback.
 *
 * Call stopAndReport() before releasing the listener handle and before
 * canceling the owner scope. Do not capture copyrighted music by default.
 */
class NativeCallbackWavCapture private constructor(
    private val playback: FfiListenerPlaybackHandle,
    private val writer: CallbackPcmWavWriter,
    scope: CoroutineScope,
    private val outputLifecycleAtStart: String,
    private val outputCountersAtStart: OutputCallbackCounters?,
) {
    private val stopRequested = AtomicBoolean(false)
    private val tapFullyDrained = AtomicBoolean(false)
    private val workerFailure = AtomicReference<Throwable?>(null)
    private var capturedFrames: ULong = 0u
    private var lostFrames: ULong = 0u
    private var callbackCount: ULong = 0u
    private val worker: Job = scope.launch(Dispatchers.IO) {
        try {
            while (isActive) {
                val batch = playback.readCallbackCapture(4096u)
                writer.append(batch.samples)
                capturedFrames = batch.capturedFrames
                lostFrames = batch.lostFrames
                callbackCount = batch.callbackCount
                if (stopRequested.get() && batch.samples.isEmpty()) {
                    tapFullyDrained.set(true)
                    break
                }
                delay(10)
            }
        } catch (error: Throwable) {
            workerFailure.set(error)
        }
    }

    suspend fun stopAndReport(): CallbackAudioCaptureReport {
        if (stopRequested.compareAndSet(false, true)) playback.endCallbackCapture()
        worker.join()
        playback.clearCallbackCapture()
        val frames = writer.finish()
        workerFailure.get()?.let { throw IllegalStateException("callback PCM capture failed", it) }
        return CallbackAudioCaptureReport(
            file = writer.file,
            wavFrames = frames,
            capturedFrames = capturedFrames,
            lostFrames = lostFrames,
            callbackCount = callbackCount,
            tapFullyDrained = tapFullyDrained.get(),
            outputLifecycleAtStart = outputLifecycleAtStart,
            outputLifecycleAtEnd = OboeBridge.lastOpenSummary(),
            outputCountersAtStart = outputCountersAtStart,
            outputCountersAtEnd = currentOutputCallbackCounters(),
        )
    }

    companion object {
        /**
         * Explicitly start a diagnostic capture in the caller's private
         * directory; no user-selected absolute path crosses the JNI boundary.
         */
        fun start(
            playback: FfiListenerPlaybackHandle,
            privateDirectory: File,
            scope: CoroutineScope,
        ): NativeCallbackWavCapture {
            require(privateDirectory.isDirectory) { "capture directory must already exist" }
            playback.startCallbackCapture(48_000u)
            var opened: CallbackPcmWavWriter? = null
            try {
                opened = CallbackPcmWavWriter(
                    File.createTempFile("silent-disco-callback-", ".wav", privateDirectory),
                )
                return NativeCallbackWavCapture(
                    playback, opened, scope, OboeBridge.lastOpenSummary(),
                    currentOutputCallbackCounters(),
                )
            } catch (error: Throwable) {
                opened?.close()
                playback.clearCallbackCapture()
                throw error
            }
        }
    }
}
