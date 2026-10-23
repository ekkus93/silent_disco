package com.ekkus.silentdisco.app

import android.app.Application
import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.BuildConfig
import com.ekkus.silentdisco.core.audio.NativeCallbackWavCapture
import kotlinx.coroutines.launch

/** Developer-only, explicitly enabled output callback recording controls. */
internal fun MainViewModel.startCallbackPcmCapture() {
    if (!BuildConfig.DEBUG) return
    if (listenerCallbackCapture != null || _callbackCaptureStatus.value == "Finalizing capture") return
    val playback = listenerPlayback ?: run {
        _callbackCaptureStatus.value = "Connect and start listener playback before recording"
        return
    }
    val directory = getApplication<Application>().getExternalFilesDir("diagnostics")
    if (directory == null) {
        _callbackCaptureStatus.value = "App-private diagnostics directory is unavailable"
        return
    }
    runCatching {
        directory.mkdirs()
        NativeCallbackWavCapture.start(playback, directory, viewModelScope)
    }.onSuccess { capture ->
        listenerCallbackCapture = capture
        _callbackCaptureStatus.value = "Recording callback PCM…"
    }.onFailure { error ->
        _callbackCaptureStatus.value = "Capture could not start: ${error.message}"
    }
}

internal fun MainViewModel.stopCallbackPcmCapture() {
    if (!BuildConfig.DEBUG) return
    val capture = listenerCallbackCapture ?: return
    listenerCallbackCapture = null
    _callbackCaptureStatus.value = "Finalizing capture"
    viewModelScope.launch {
        runCatching { capture.stopAndReport() }
            .onSuccess { report ->
                _callbackCaptureStatus.value = if (report.complete && report.noUnexpectedOutputReopen == true) {
                    "Capture saved: ${report.file.name} (${report.wavFrames} frames; " +
                        "${report.callbackCount} callbacks; no reopen). Location: ${report.file.parent}"
                } else {
                    "Unqualified capture: ${report.file.name}; ${report.lostFrames} dropped frames, " +
                        "${report.wavFrames} written, ${report.capturedFrames} captured; " +
                        "no output reopen=${report.noUnexpectedOutputReopen}. " +
                        "Do not count this recording as gapless acceptance."
                }
            }
            .onFailure { error ->
                _callbackCaptureStatus.value = "Capture failed: ${error.message}"
            }
    }
}
