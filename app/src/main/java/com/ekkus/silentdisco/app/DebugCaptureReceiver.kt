package com.ekkus.silentdisco.app

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/**
 * Debug-only control for the listener PCM capture, so a recording can be started and stopped
 * while playback runs (the capture controls otherwise live behind the start screen's Settings).
 *
 * Registered only in debug builds. Usage:
 *   adb shell am broadcast -a com.ekkus.silentdisco.debug.PCM_CAPTURE --es command start
 *   adb shell am broadcast -a com.ekkus.silentdisco.debug.PCM_CAPTURE --es command stop
 */
internal class DebugCaptureReceiver(private val viewModel: MainViewModel) : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != ACTION) return
        when (intent.getStringExtra(EXTRA_COMMAND)) {
            "start" -> viewModel.startCallbackPcmCapture()
            "stop" -> viewModel.stopCallbackPcmCapture()
        }
    }

    companion object {
        const val ACTION = "com.ekkus.silentdisco.debug.PCM_CAPTURE"
        const val EXTRA_COMMAND = "command"
    }
}
