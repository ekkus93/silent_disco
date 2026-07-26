package com.ekkus.silentdisco.platform.persistence

import android.content.Context
import android.content.SharedPreferences
import com.ekkus.silentdisco.core.persistence.LegacyPreferencesContract
import com.ekkus.silentdisco.core.rust.RustDatabase
import com.ekkus.silentdisco.core.rust.RustDatabaseBridge
import com.ekkus.silentdisco.core.rust.RustLegacyImportOutcome
import com.ekkus.silentdisco.core.rust.RustStoredTuningSettings
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

class AndroidRustDomainStoreException(
    message: String,
    cause: Throwable? = null,
) : IllegalStateException(message, cause)

/**
 * Android coordinator for the Rust-owned domain database.
 *
 * This class may read only the documented legacy preference keys. It never
 * opens SQLite and never retains a preference fallback after Rust initializes.
 */
class AndroidRustDomainStore internal constructor(
    context: Context,
    private val pathProvider: AndroidDatabasePathProvider = AndroidDatabasePathProvider(context),
    preferencesName: String = DEFAULT_PREFERENCES_NAME,
    private val ioDispatcher: CoroutineDispatcher = Dispatchers.IO,
) {
    private val preferences: SharedPreferences = context.getSharedPreferences(
        preferencesName,
        Context.MODE_PRIVATE,
    )
    private val mutex = Mutex()
    private var database: RustDatabase? = null

    suspend fun initialize(): RustStoredTuningSettings = withContext(ioDispatcher) {
        mutex.withLock {
            database?.let { existing ->
                return@withLock existing.loadSettings()
                    ?: throw AndroidRustDomainStoreException(
                        "Rust database is open but contains no settings row",
                    )
            }

            val opened = RustDatabaseBridge.open(pathProvider.databasePath())
            try {
                val legacy = readLegacySnapshot()
                val outcome = opened.importLegacy(
                    settings = legacy.settings,
                    trustedDeviceIds = legacy.trustedDeviceIds.toTypedArray(),
                )
                clearLegacyKeysAfterCommittedImport(legacy.keysToDelete, outcome)
                val settings = opened.loadSettings()
                    ?: throw AndroidRustDomainStoreException(
                        "Rust legacy import committed without a settings row",
                    )
                database = opened
                settings
            } catch (error: Throwable) {
                runCatching { opened.close() }
                throw error
            }
        }
    }

    suspend fun saveTuning(settings: RustStoredTuningSettings) = withContext(ioDispatcher) {
        mutex.withLock {
            requireDatabase().saveSettings(settings)
        }
    }

    suspend fun trustDevice(
        deviceId: String,
        displayName: String,
        observedAtMs: Long = System.currentTimeMillis(),
    ) = withContext(ioDispatcher) {
        mutex.withLock {
            requireDatabase().trustDevice(deviceId, displayName, observedAtMs)
        }
    }

    suspend fun isTrusted(deviceId: String): Boolean = withContext(ioDispatcher) {
        mutex.withLock {
            requireDatabase().isTrusted(deviceId)
        }
    }

    suspend fun close() = withContext(ioDispatcher) {
        mutex.withLock {
            val current = database ?: return@withLock
            current.close()
            database = null
        }
    }

    internal fun databasePathForTests(): String = pathProvider.databasePath()

    private fun requireDatabase(): RustDatabase = database
        ?: throw AndroidRustDomainStoreException(
            "Rust domain database has not initialized successfully",
        )

    private fun readLegacySnapshot(): LegacySnapshot {
        val now = System.currentTimeMillis()
        val trustedKeys = preferences.all
            .asSequence()
            .mapNotNull { (key, value) ->
                val deviceId = LegacyPreferencesContract.trustedDeviceId(key)
                if (deviceId != null && value == true) key to deviceId else null
            }
            .toList()
        val settings = RustStoredTuningSettings(
            syncSampleWindow = preferences.getInt(
                LegacyPreferencesContract.SYNC_SAMPLE_WINDOW,
                12,
            ),
            syncCadenceMs = preferences.getLong(
                LegacyPreferencesContract.SYNC_CADENCE_MS,
                2_000L,
            ),
            startupBufferMs = preferences.getLong(
                LegacyPreferencesContract.STARTUP_BUFFER_MS,
                400L,
            ),
            latePacketThresholdMs = preferences.getLong(
                LegacyPreferencesContract.LATE_PACKET_THRESHOLD_MS,
                40L,
            ),
            hardResyncThresholdMs = preferences.getLong(
                LegacyPreferencesContract.HARD_RESYNC_THRESHOLD_MS,
                120L,
            ),
            syncDriftThresholdMs = Double.fromBits(
                preferences.getLong(
                    LegacyPreferencesContract.SYNC_DRIFT_THRESHOLD_BITS,
                    18.0.toBits(),
                ),
            ),
            scanWindowMs = 3_000L,
            updatedAtMs = now,
        )
        return LegacySnapshot(
            settings = settings,
            trustedDeviceIds = trustedKeys.map { it.second },
            keysToDelete = LegacyPreferencesContract.tuningKeys + trustedKeys.map { it.first },
        )
    }

    private fun clearLegacyKeysAfterCommittedImport(
        keys: Set<String>,
        outcome: RustLegacyImportOutcome,
    ) {
        check(
            outcome == RustLegacyImportOutcome.IMPORTED ||
                outcome == RustLegacyImportOutcome.ALREADY_IMPORTED,
        ) {
            "legacy keys cannot be deleted before Rust reports a committed import"
        }
        if (keys.isEmpty()) return
        val editor = preferences.edit()
        keys.forEach(editor::remove)
        if (!editor.commit()) {
            throw AndroidRustDomainStoreException(
                "Rust import committed, but Android could not delete legacy preference keys; " +
                    "startup will retry cleanup",
            )
        }
    }

    private data class LegacySnapshot(
        val settings: RustStoredTuningSettings,
        val trustedDeviceIds: List<String>,
        val keysToDelete: Set<String>,
    )

    companion object {
        const val DEFAULT_PREFERENCES_NAME = "silent-disco"
    }
}
