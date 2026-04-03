package com.ekkus.silentdisco.core.logging

import android.util.Log

class AppLogger(private val tag: String = "SilentDisco") {
    fun d(event: String, message: String) = Log.d(tag, "[$event] $message")
    fun i(event: String, message: String) = Log.i(tag, "[$event] $message")
    fun w(event: String, message: String) = Log.w(tag, "[$event] $message")
    fun e(event: String, message: String, throwable: Throwable? = null) =
        Log.e(tag, "[$event] $message", throwable)
}
