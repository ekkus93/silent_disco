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
    val outputLifecycleAtStart: String,
    val outputLifecycleAtEnd: String,
) {
    val complete: Boolean get() = lostFrames == 0uL && wavFrames.toULong() == capturedFrames
    /** Unknown output state cannot count as a successful no-reopen check. */
    val noUnexpectedOutputReopen: Boolean?
        get() {
            val initial = parseOutputLifecycle(outputLifecycleAtStart) ?: return null
            val final = parseOutputLifecycle(outputLifecycleAtEnd) ?: return null
            return initial.opens == final.opens
        }
}

internal data class OutputLifecycle(val opens: Int, val rebinds: Int, val closes: Int)

internal fun parseOutputLifecycle(summary: String): OutputLifecycle? {
    val tokens = summary.split(' ')
    fun number(key: String): Int? =
        tokens.firstOrNull { it.startsWith("$key=") }?.substringAfter('=')?.toIntOrNull()
    return OutputLifecycle(
        opens = number("opens") ?: return null,
        rebinds = number("rebinds") ?: return null,
        closes = number("closes") ?: return null,
    )
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
) {
    private val stopRequested = AtomicBoolean(false)
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
                if (stopRequested.get() && batch.samples.isEmpty()) break
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
            outputLifecycleAtStart = outputLifecycleAtStart,
            outputLifecycleAtEnd = OboeBridge.lastOpenSummary(),
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
                return NativeCallbackWavCapture(playback, opened, scope, OboeBridge.lastOpenSummary())
            } catch (error: Throwable) {
                opened?.close()
                playback.clearCallbackCapture()
                throw error
            }
        }
    }
}
