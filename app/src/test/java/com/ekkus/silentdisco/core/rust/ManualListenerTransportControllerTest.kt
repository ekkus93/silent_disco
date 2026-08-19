package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.model.ManualConnectUiState
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportException
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class ManualListenerTransportControllerTest {
    @Test
    fun everyPostConnectionExceptionMapsToDisconnectedNotFailed() {
        val exceptions = listOf(
            FfiListenerTransportException.InvalidEndpoint("invalid"),
            FfiListenerTransportException.UnsupportedProtocolVersion("version"),
            FfiListenerTransportException.Unauthorized("unauthorized"),
            FfiListenerTransportException.Timeout("timeout"),
            FfiListenerTransportException.ConnectionFailed("connection failed"),
            FfiListenerTransportException.Io("io"),
            FfiListenerTransportException.Closed("transport event channel is closed"),
        )

        for (exception in exceptions) {
            val state = mapPostConnectionFailure(exception)
            assertThat(state).isEqualTo(ManualConnectUiState.Disconnected(exception.message))
        }
    }

    @Test
    fun closedExceptionIsNotRenderedAsAConnectionFailure() {
        val state = mapPostConnectionFailure(
            FfiListenerTransportException.Closed("transport event channel is closed"),
        )

        assertThat(state).isNotInstanceOf(ManualConnectUiState.Failed::class.java)
    }

    @Test
    fun transportClockTranslationUsesElapsedDeltaFromCapturedOrigin() {
        val translated = translateTransportElapsedToPumpClock(
            transportClockOriginMs = 10_000uL,
            elapsedTransportMs = 10_275uL,
            fallbackNowMs = { error("fallback must not be used when an origin exists") },
        )

        assertThat(translated).isEqualTo(275uL)
    }

    @Test
    fun transportClockTranslationFallsBackToLivePumpClockWithoutAnOrigin() {
        val translated = translateTransportElapsedToPumpClock(
            transportClockOriginMs = null,
            elapsedTransportMs = 99_999uL,
            fallbackNowMs = { 4321uL },
        )

        assertThat(translated).isEqualTo(4321uL)
    }

    @Test
    fun transportClockTranslationSaturatesWhenReceiptPredatesCapturedOrigin() {
        val translated = translateTransportElapsedToPumpClock(
            transportClockOriginMs = 10_000uL,
            elapsedTransportMs = 9_999uL,
            fallbackNowMs = { error("fallback must not be used when an origin exists") },
        )

        assertThat(translated).isEqualTo(0uL)
    }
    @Test
    fun cleanupFailureAggregationPreservesTheFirstFailureAndSuppressesLaterOnes() {
        val first = IllegalStateException("first")
        val second = IllegalArgumentException("second")

        val aggregated = mergeManualCleanupFailure(null, first)
        val afterSecond = mergeManualCleanupFailure(aggregated, second)

        assertThat(afterSecond).isSameInstanceAs(first)
        assertThat(afterSecond.suppressed.toList()).containsExactly(second)
    }

}
