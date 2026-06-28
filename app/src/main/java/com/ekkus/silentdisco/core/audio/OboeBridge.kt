package com.ekkus.silentdisco.core.audio

object OboeBridge {
    val loadResult: Result<Unit> = runCatching { System.loadLibrary("silentdisco") }
    val isAvailable: Boolean get() = loadResult.isSuccess

    external fun nativeGetAudioBackend(): String
    external fun nativeGetAudioStatus(): String

    fun backendSummary(): String = if (isAvailable) {
        runCatching { nativeGetAudioBackend() }.getOrDefault("No native Oboe")
    } else {
        "Native library not loaded: ${loadResult.exceptionOrNull()?.message ?: "unknown error"}"
    }

    fun statusSummary(): String = if (isAvailable) {
        runCatching { nativeGetAudioStatus() }.getOrDefault("Native bridge unavailable")
    } else {
        "Unavailable — native library did not load"
    }
}
