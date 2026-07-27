package com.ekkus.silentdisco.core.logging

import android.os.SystemClock
import android.util.Log
import java.io.PrintStream

class AppLogger(private val tag: String = "SilentDisco") {
    fun d(event: String, message: String): Int =
        write("DEBUG", event, message) { formatted -> Log.d(tag, formatted) }

    fun i(event: String, message: String): Int =
        write("INFO", event, message) { formatted -> Log.i(tag, formatted) }

    fun w(event: String, message: String): Int =
        write("WARN", event, message) { formatted -> Log.w(tag, formatted) }

    fun e(event: String, message: String, throwable: Throwable? = null): Int =
        write("ERROR", event, message, throwable) { formatted ->
            Log.e(tag, formatted, throwable)
        }

    private inline fun write(
        level: String,
        event: String,
        message: String,
        throwable: Throwable? = null,
        androidLog: (String) -> Int,
    ): Int {
        val formatted = format(event, message)
        return try {
            androidLog(formatted)
        } catch (loggingFailure: RuntimeException) {
            writeFallback(
                level = level,
                formatted = formatted,
                throwable = throwable,
                loggingFailure = loggingFailure,
            )
            0
        }
    }

    private fun format(event: String, message: String): String =
        "[t=${monotonicTimeMs()}][$event] $message"

    private fun monotonicTimeMs(): Long =
        try {
            SystemClock.elapsedRealtime()
        } catch (_: RuntimeException) {
            System.nanoTime() / 1_000_000L
        }

    private fun writeFallback(
        level: String,
        formatted: String,
        throwable: Throwable?,
        loggingFailure: RuntimeException,
        output: PrintStream = System.err,
    ) {
        output.println("[$tag][$level] $formatted")
        throwable?.printStackTrace(output)
        output.println(
            "[$tag][LOGGER_FAILURE] Android logging was unavailable: " +
                "${loggingFailure::class.java.simpleName}: ${loggingFailure.message}",
        )
    }
}
