package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import org.junit.Assert.assertThrows
import org.junit.Test

class RustCoreBridgeTest {
    @Test
    fun supportedAbiVersionIsReturned() {
        assertThat(validateRustCoreAbiVersion(1)).isEqualTo(1)
    }

    @Test
    fun olderAbiVersionFailsExplicitly() {
        val error = assertThrows(UnsupportedRustCoreAbiException::class.java) {
            validateRustCoreAbiVersion(0)
        }

        assertThat(error.actualVersion).isEqualTo(0)
        assertThat(error.message).contains("expected 1")
    }

    @Test
    fun newerAbiVersionFailsExplicitly() {
        val error = assertThrows(UnsupportedRustCoreAbiException::class.java) {
            validateRustCoreAbiVersion(2)
        }

        assertThat(error.actualVersion).isEqualTo(2)
        assertThat(error.message).contains("Unsupported Rust core ABI version 2")
    }

    @Test
    fun stableSynchronizationConfidenceCodesDecodeWithoutPresentationLabels() {
        assertThat(RustSyncConfidence.fromStableCode(1)).isEqualTo(RustSyncConfidence.UNKNOWN)
        assertThat(RustSyncConfidence.fromStableCode(5)).isEqualTo(RustSyncConfidence.EXCELLENT)
    }

    @Test
    fun unknownSynchronizationConfidenceFailsExplicitly() {
        val error = assertThrows(RustSyncBridgeProtocolException::class.java) {
            RustSyncConfidence.fromStableCode(99)
        }

        assertThat(error.message).contains("unsupported confidence code 99")
    }

    @Test
    fun nonFiniteNativeDoubleFailsInsteadOfBecomingAStateValue() {
        val error = assertThrows(RustSyncBridgeProtocolException::class.java) {
            decodeFiniteNativeDouble(
                fieldName = "test offset",
                bits = Double.NaN.toBits(),
            )
        }

        assertThat(error.message).contains("non-finite test offset")
    }
}
