package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiBusyJoinRetry

/**
 * Applies Rust's busy-refusal rule to one manual connection (remediation F3):
 * a join the host refused as busy is retried at most as many times as Rust
 * allows, with Rust's delays; every other refusal is final. The policy (what
 * counts as busy, how many retries, how long to wait) comes from Rust; this
 * only counts the retries of the current connection.
 */
class BusyJoinRetries(
    private val isBusy: (String) -> Boolean = { com.ekkus.silentdisco.core.uniffi.isHostBusyRejection(it) },
    private val nextRetry: (UByte) -> FfiBusyJoinRetry? = { com.ekkus.silentdisco.core.uniffi.nextBusyJoinRetry(it) },
) {
    sealed interface Decision {
        /** Connect again after [delayMs]; this is retry [attempt] of [maxAttempts]. */
        data class Retry(val attempt: Int, val maxAttempts: Int, val delayMs: Long) : Decision

        /** The refusal stands; show [reason]. */
        data class Final(val reason: String) : Decision
    }

    private var retriesSoFar: Int = 0

    /** A new connection the user asked for starts with no retries used. */
    fun reset() {
        retriesSoFar = 0
    }

    fun onRejected(reason: String): Decision {
        if (!isBusy(reason)) {
            retriesSoFar = 0
            return Decision.Final(reason)
        }
        val retry = nextRetry(retriesSoFar.toUByte())
        if (retry == null) {
            val tried = retriesSoFar
            retriesSoFar = 0
            return Decision.Final("The host is busy (tried $tried more times); try again later.")
        }
        retriesSoFar = retry.attempt.toInt()
        return Decision.Retry(retry.attempt.toInt(), retry.maxAttempts.toInt(), retry.delayMs.toLong())
    }
}
