package com.ekkus.silentdisco.core.audio

/**
 * JNI bridge to the native `silentdisco` library, which owns one Oboe output
 * stream reading from the shared Rust render ring via the narrow real-time C
 * ABI (`include/silent_disco_audio.h`). Every `nativeOboe*` function here is
 * non-real-time control-plane glue; the audio callback itself lives entirely
 * in C++ and never crosses back into JNI.
 */
object OboeBridge {
    val loadResult: Result<Unit> = runCatching { System.loadLibrary("silentdisco") }
    val isAvailable: Boolean get() = loadResult.isSuccess

    external fun nativeGetAudioBackend(): String
    external fun nativeGetAudioStatus(): String

    /** Opens the native Oboe output stream bound to `engineToken`; returns an `OboeAdapterStatus` code. */
    external fun nativeOboeOpen(engineToken: Long): Int
    /**
     * Points the already-open stream at a different render-ring engine token
     * without reopening it; returns an `OboeAdapterStatus` code, `NotOpen`
     * (-7) when there is no stream to rebind. Used for a track change, so a
     * session keeps the one output stream it was originally granted.
     */
    external fun nativeOboeRebind(engineToken: Long): Int

    external fun nativeOboeClose()
    external fun nativeOboeIsOpen(): Boolean
    external fun nativeOboeActualSampleRate(): Int
    external fun nativeOboeActualChannelCount(): Int

    /** Returns and clears the last fatal status observed by the real-time callback, or 0 if none. */
    external fun nativeOboeTakeFatalStatus(): Int

    /** Returns and clears whether the stream disconnected (e.g. a route change) since the last check. */
    external fun nativeOboeTakeDisconnected(): Boolean

    /** Cumulative reads that had to silence-fill at least one frame; 0 if not open. */
    external fun nativeOboeUnderrunCount(): Long

    /** Cumulative frames filled with silence because the ring lacked data; 0 if not open. */
    external fun nativeOboeSilenceFilledFrames(): Long

    /** Cumulative frames actually rendered from real ring contents; 0 if not open. */
    external fun nativeOboeFramesRendered(): Long

    /**
     * Configuration the device actually granted on the most recent open,
     * retained after close so a finished stream's output path is still
     * diagnosable. The live accessors above report 0 once closed, which on a
     * device without logcat leaves no way to tell whether a stream was
     * granted what it asked for.
     */
    external fun nativeOboeLastOpenSummary(): String

    fun backendSummary(): String = if (isAvailable) {
        runCatching { nativeGetAudioBackend() }.getOrDefault("No native Oboe")
    } else {
        "Native library not loaded: ${loadResult.exceptionOrNull()?.message ?: "unknown error"}"
    }

    fun lastOpenSummary(): String = if (isAvailable) {
        runCatching { nativeOboeLastOpenSummary() }.getOrDefault("unavailable")
    } else {
        "Unavailable — native library did not load"
    }

    fun statusSummary(): String = if (isAvailable) {
        runCatching { nativeGetAudioStatus() }.getOrDefault("Native bridge unavailable")
    } else {
        "Unavailable — native library did not load"
    }
}
