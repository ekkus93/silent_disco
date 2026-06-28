package com.ekkus.silentdisco.core.audio

object OboeBridge {
    init {
        runCatching {
            System.loadLibrary("silentdisco")
        }
    }

    external fun nativeGetAudioBackend(): String
    external fun nativeGetAudioStatus(): String

    fun backendSummary(): String = runCatching { nativeGetAudioBackend() }.getOrDefault("No native Oboe")
    fun statusSummary(): String = runCatching { nativeGetAudioStatus() }.getOrDefault("Native bridge unavailable")
}
