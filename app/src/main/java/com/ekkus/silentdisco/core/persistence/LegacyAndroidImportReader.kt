package com.ekkus.silentdisco.core.persistence

import android.content.SharedPreferences
import com.ekkus.silentdisco.core.rust.LEGACY_ANDROID_IMPORT_VERSION
import com.ekkus.silentdisco.core.rust.RustLegacyAndroidImport
import com.ekkus.silentdisco.core.rust.RustStoredSettings
import com.ekkus.silentdisco.core.rust.RustTrustedDevice

private const val TRUSTED_DEVICE_PREFIX = "trusted:"

private val tuningKeys = setOf(
    LegacyPreferencesContract.SYNC_SAMPLE_WINDOW,
    LegacyPreferencesContract.SYNC_CADENCE_MS,
    LegacyPreferencesContract.STARTUP_BUFFER_MS,
    LegacyPreferencesContract.LATE_PACKET_THRESHOLD_MS,
    LegacyPreferencesContract.HARD_RESYNC_THRESHOLD_MS,
    LegacyPreferencesContract.SYNC_DRIFT_THRESHOLD_BITS,
)

data class LegacyAndroidImportSnapshot(
    val request: RustLegacyAndroidImport,
    val keysToDeleteAfterCommit: Set<String>,
)

class LegacyPreferencesReadException(
    message: String,
) : IllegalStateException(message)

class LegacyPreferencesCleanupException(
    message: String,
) : IllegalStateException(message)

class LegacyAndroidImportReader(
    private val clock: () -> Long = System::currentTimeMillis,
) {
    fun read(preferences: SharedPreferences): LegacyAndroidImportSnapshot =
        buildLegacyAndroidImportSnapshot(preferences.all, clock())

    fun deleteCommittedKeys(
        preferences: SharedPreferences,
        snapshot: LegacyAndroidImportSnapshot,
    ) {
        if (snapshot.keysToDeleteAfterCommit.isEmpty()) return
        val editor = preferences.edit()
        snapshot.keysToDeleteAfterCommit.sorted().forEach(editor::remove)
        if (!editor.commit()) {
            throw LegacyPreferencesCleanupException(
                "Rust committed the legacy import, but Android could not durably remove the imported SharedPreferences keys",
            )
        }
    }
}

internal fun buildLegacyAndroidImportSnapshot(
    values: Map<String, *>,
    importedAtMs: Long,
): LegacyAndroidImportSnapshot {
    require(importedAtMs >= 0L) { "importedAtMs must not be negative" }
    val presentTuningKeys = tuningKeys.filterTo(linkedSetOf()) { values.containsKey(it) }
    val settings = if (presentTuningKeys.isEmpty()) {
        null
    } else {
        RustStoredSettings(
            syncSampleWindow = values.legacyInt(LegacyPreferencesContract.SYNC_SAMPLE_WINDOW, 12),
            syncCadenceMs = values.legacyLong(LegacyPreferencesContract.SYNC_CADENCE_MS, 2_000L),
            startupBufferMs = values.legacyLong(LegacyPreferencesContract.STARTUP_BUFFER_MS, 400L),
            latePacketThresholdMs = values.legacyLong(LegacyPreferencesContract.LATE_PACKET_THRESHOLD_MS, 40L),
            hardResyncThresholdMs = values.legacyLong(LegacyPreferencesContract.HARD_RESYNC_THRESHOLD_MS, 120L),
            syncDriftThresholdMs = Double.fromBits(
                values.legacyLong(
                    LegacyPreferencesContract.SYNC_DRIFT_THRESHOLD_BITS,
                    18.0.toBits(),
                ),
            ),
            scanWindowMs = 3_000L,
            updatedAtMs = importedAtMs,
        )
    }
    val trustedKeys = values.keys
        .filter { it.startsWith(TRUSTED_DEVICE_PREFIX) }
        .sorted()
    val trustedDevices = trustedKeys.mapNotNull { key ->
        val trusted = values[key] as? Boolean ?: throw LegacyPreferencesReadException(
            "Legacy SharedPreferences key '$key' has type ${values[key]?.javaClass?.name ?: "null"}; expected Boolean",
        )
        if (!trusted) return@mapNotNull null
        val deviceId = key.removePrefix(TRUSTED_DEVICE_PREFIX)
        RustTrustedDevice(
            deviceId = deviceId,
            displayName = deviceId,
            trustState = "trusted",
            firstSeenMs = importedAtMs,
            lastSeenMs = importedAtMs,
            updatedAtMs = importedAtMs,
        )
    }
    return LegacyAndroidImportSnapshot(
        request = RustLegacyAndroidImport(
            version = LEGACY_ANDROID_IMPORT_VERSION,
            importedAtMs = importedAtMs,
            settings = settings,
            trustedDevices = trustedDevices,
        ),
        keysToDeleteAfterCommit = presentTuningKeys + trustedKeys,
    )
}

private fun Map<String, *>.legacyInt(key: String, defaultValue: Int): Int {
    if (!containsKey(key)) return defaultValue
    return this[key] as? Int ?: throw LegacyPreferencesReadException(
        "Legacy SharedPreferences key '$key' has type ${this[key]?.javaClass?.name ?: "null"}; expected Int",
    )
}

private fun Map<String, *>.legacyLong(key: String, defaultValue: Long): Long {
    if (!containsKey(key)) return defaultValue
    return this[key] as? Long ?: throw LegacyPreferencesReadException(
        "Legacy SharedPreferences key '$key' has type ${this[key]?.javaClass?.name ?: "null"}; expected Long",
    )
}
