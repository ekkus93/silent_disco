from pathlib import Path

Path("app/src/test/java/com/ekkus/silentdisco/core/persistence/AndroidDatabasePathProviderTest.kt").write_text(r'''package com.ekkus.silentdisco.core.persistence

import com.google.common.truth.Truth.assertThat
import java.nio.file.Files
import org.junit.Assert.assertThrows
import org.junit.Test

class AndroidDatabasePathProviderTest {
    @Test
    fun createsOnlyTheNoBackupParentAndReturnsTheCompleteDatabasePath() {
        val root = Files.createTempDirectory("silent-disco-no-backup")
        try {
            val database = AndroidDatabasePathProvider.resolveNoBackupPath(root.toFile())

            assertThat(database).isEqualTo(root.resolve("databases/silent-disco.sqlite3").toFile())
            assertThat(root.resolve("databases").toFile().isDirectory).isTrue()
            assertThat(database.exists()).isFalse()
        } finally {
            root.toFile().deleteRecursively()
        }
    }

    @Test
    fun rejectsAParentPathOccupiedByARegularFile() {
        val root = Files.createTempDirectory("silent-disco-no-backup-file")
        try {
            Files.write(root.resolve("databases"), "not a directory".toByteArray())
            assertThrows(AndroidDatabasePathException::class.java) {
                AndroidDatabasePathProvider.resolveNoBackupPath(root.toFile())
            }
        } finally {
            root.toFile().deleteRecursively()
        }
    }
}
''', encoding="utf-8")

Path("app/src/test/java/com/ekkus/silentdisco/core/persistence/LegacyAndroidImportReaderTest.kt").write_text(r'''package com.ekkus.silentdisco.core.persistence

import com.google.common.truth.Truth.assertThat
import org.junit.Assert.assertThrows
import org.junit.Test

class LegacyAndroidImportReaderTest {
    @Test
    fun selectsOnlyKnownTuningAndTrustKeys() {
        val values = mapOf<String, Any>(
            LegacyPreferencesContract.SYNC_SAMPLE_WINDOW to 16,
            LegacyPreferencesContract.SYNC_CADENCE_MS to 2_500L,
            LegacyPreferencesContract.SYNC_DRIFT_THRESHOLD_BITS to 24.0.toBits(),
            "trusted:listener-a" to true,
            "trusted:listener-b" to false,
            "unrelated-key" to "preserve me",
        )

        val snapshot = buildLegacyAndroidImportSnapshot(values, importedAtMs = 9_000L)

        assertThat(snapshot.request.settings?.syncSampleWindow).isEqualTo(16)
        assertThat(snapshot.request.settings?.syncCadenceMs).isEqualTo(2_500L)
        assertThat(snapshot.request.settings?.syncDriftThresholdMs).isEqualTo(24.0)
        assertThat(snapshot.request.settings?.scanWindowMs).isEqualTo(3_000L)
        assertThat(snapshot.request.trustedDevices.map { it.deviceId }).containsExactly("listener-a")
        assertThat(snapshot.keysToDeleteAfterCommit).containsExactly(
            LegacyPreferencesContract.SYNC_SAMPLE_WINDOW,
            LegacyPreferencesContract.SYNC_CADENCE_MS,
            LegacyPreferencesContract.SYNC_DRIFT_THRESHOLD_BITS,
            "trusted:listener-a",
            "trusted:listener-b",
        )
    }

    @Test
    fun emptyLegacyMapStillProducesAnExplicitVersionedImport() {
        val snapshot = buildLegacyAndroidImportSnapshot(
            values = emptyMap<String, Any>(),
            importedAtMs = 1L,
        )

        assertThat(snapshot.request.settings).isNull()
        assertThat(snapshot.request.trustedDevices).isEmpty()
        assertThat(snapshot.keysToDeleteAfterCommit).isEmpty()
    }

    @Test
    fun rejectsLegacyTypeMismatchWithoutProducingCleanupKeys() {
        assertThrows(LegacyPreferencesReadException::class.java) {
            buildLegacyAndroidImportSnapshot(
                mapOf(LegacyPreferencesContract.SYNC_CADENCE_MS to 2_000),
                importedAtMs = 1L,
            )
        }
        assertThrows(LegacyPreferencesReadException::class.java) {
            buildLegacyAndroidImportSnapshot(
                mapOf("trusted:listener-a" to "true"),
                importedAtMs = 1L,
            )
        }
    }
}
''', encoding="utf-8")

Path("app/src/test/java/com/ekkus/silentdisco/core/rust/RustStorageBridgeJsonTest.kt").write_text(r'''package com.ekkus.silentdisco.core.rust

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
''', encoding="utf-8")
