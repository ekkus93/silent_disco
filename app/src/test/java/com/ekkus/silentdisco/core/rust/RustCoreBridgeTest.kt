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
}
