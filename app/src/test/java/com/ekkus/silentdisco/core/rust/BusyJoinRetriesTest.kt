package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiBusyJoinRetry
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class BusyJoinRetriesTest {
    // The shape of Rust's policy (JVM tests cannot load it): three retries, 1 s, 2 s, 4 s.
    private val retries = BusyJoinRetries(
        isBusy = { it == "host_busy" },
        nextRetry = { done ->
            if (done >= 3u) null else FfiBusyJoinRetry(
                attempt = (done + 1u).toUByte(),
                maxAttempts = 3u,
                delayMs = 1_000uL shl done.toInt(),
            )
        },
    )

    @Test
    fun aBusyRefusalIsRetriedThreeTimesThenFinal() {
        assertThat((1..4).map { retries.onRejected("host_busy") }).containsExactly(
            BusyJoinRetries.Decision.Retry(attempt = 1, maxAttempts = 3, delayMs = 1_000),
            BusyJoinRetries.Decision.Retry(attempt = 2, maxAttempts = 3, delayMs = 2_000),
            BusyJoinRetries.Decision.Retry(attempt = 3, maxAttempts = 3, delayMs = 4_000),
            BusyJoinRetries.Decision.Final("The host is busy (tried 3 more times); try again later."),
        ).inOrder()
    }

    @Test
    fun anyOtherRefusalIsFinalAtOnce() {
        assertThat(retries.onRejected("device_not_approved"))
            .isEqualTo(BusyJoinRetries.Decision.Final("device_not_approved"))
    }

    @Test
    fun aNewConnectionStartsWithNoRetriesUsed() {
        retries.onRejected("host_busy")
        retries.onRejected("host_busy")
        retries.reset()
        assertThat(retries.onRejected("host_busy"))
            .isEqualTo(BusyJoinRetries.Decision.Retry(attempt = 1, maxAttempts = 3, delayMs = 1_000))
    }
}
