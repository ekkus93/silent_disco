package com.ekkus.silentdisco.core.logging

import android.os.SystemClock
import android.util.Log

class AppLogger(private val tag: String = "SilentDisco") {
    fun d(event: String, message: String) = Log.d(tag, format(event, message))
    fun i(event: String, message: String) = Log.i(tag, format(event, message))
    fun w(event: String, message: String) = Log.w(tag, format(event, message))
    fun e(event: String, message: String, throwable: Throwable? = null) =
        Log.e(tag, format(event, message), throwable)

    private fun format(event: String, message: String): String =
        "[t=${SystemClock.elapsedRealtime()}][$event] $message"
}
