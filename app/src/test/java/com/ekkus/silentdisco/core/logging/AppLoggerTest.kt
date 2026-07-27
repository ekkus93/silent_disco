package com.ekkus.silentdisco.core.logging

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class AppLoggerTest {
    @Test
    fun androidLoggingUnavailableDoesNotBreakCaller() {
        val logger = AppLogger(tag = "TestLogger")
        val cause = IllegalStateException("original failure")

        assertThat(logger.d("debug-event", "debug message")).isEqualTo(0)
        assertThat(logger.i("info-event", "info message")).isEqualTo(0)
        assertThat(logger.w("warning-event", "warning message")).isEqualTo(0)
        assertThat(logger.e("error-event", "error message", cause)).isEqualTo(0)
    }
}
