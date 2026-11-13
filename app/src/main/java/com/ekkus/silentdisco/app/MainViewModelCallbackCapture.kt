package com.ekkus.silentdisco.app

import android.app.Application

/** Developer-only, explicitly enabled output callback recording (owned by [diagnostics]). */
internal fun MainViewModel.startCallbackPcmCapture() {
    // The manual listener path owns its playback runtime through its own controller.
    diagnostics.startCapture(
        playback = listenerPlayback ?: manualListenerController.playbackRuntime,
        directory = getApplication<Application>().getExternalFilesDir("diagnostics"),
    )
}

internal fun MainViewModel.stopCallbackPcmCapture() = diagnostics.stopCapture()
