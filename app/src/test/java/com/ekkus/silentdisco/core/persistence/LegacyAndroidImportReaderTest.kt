package com.ekkus.silentdisco.core.persistence

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
