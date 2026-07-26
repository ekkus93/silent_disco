package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import org.junit.Assert.assertThrows
import org.junit.Test

class RustStorageBridgeJsonTest {
    @Test
    fun decodesSuccessfulAndNullableResults() {
        val settings = decodeRustStorageEnvelope(
            """{"ok":true,"result":{"syncSampleWindow":12,"syncCadenceMs":2000,"startupBufferMs":400,"latePacketThresholdMs":40,"hardResyncThresholdMs":120,"syncDriftThresholdMs":18.0,"scanWindowMs":3000,"updatedAtMs":100},"error":null}""",
            RustStoredSettings.serializer(),
        )
        assertThat(settings?.syncCadenceMs).isEqualTo(2_000L)

        assertThat(
            decodeRustStorageEnvelope(
                """{"ok":true,"result":null,"error":null}""",
                RustStoredSettings.serializer(),
            ),
        ).isNull()
    }

    @Test
    fun surfacesStructuredRustFailureWithoutConvertingItToDefaults() {
        val error = assertThrows(RustStorageException::class.java) {
            decodeRustStorageEnvelope(
                """{"ok":false,"result":null,"error":{"code":"storage_corruption","operation":"open","message":"integrity check failed","retryable":false,"coreRemainsUsable":false,"schemaVersion":2}}""",
                RustStoredSettings.serializer(),
            )
        }
        assertThat(error.code).isEqualTo("storage_corruption")
        assertThat(error.operation).isEqualTo("open")
        assertThat(error.schemaVersion).isEqualTo(2)
        assertThat(error.coreRemainsUsable).isFalse()
    }

    @Test
    fun rejectsUnknownProtocolFields() {
        assertThrows(RustStorageBridgeProtocolException::class.java) {
            decodeRustStorageEnvelope(
                """{"ok":true,"result":null,"error":null,"unexpected":true}""",
                RustStoredSettings.serializer(),
            )
        }
    }
}
