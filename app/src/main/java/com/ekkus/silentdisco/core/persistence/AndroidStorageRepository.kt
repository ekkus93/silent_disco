package com.ekkus.silentdisco.core.persistence

import android.content.Context
import com.ekkus.silentdisco.core.rust.LEGACY_ANDROID_IMPORT_VERSION
import com.ekkus.silentdisco.core.rust.RustLegacyImportOutcome
import com.ekkus.silentdisco.core.rust.RustStorageBridge
import com.ekkus.silentdisco.core.rust.RustStorageSession
import com.ekkus.silentdisco.core.rust.RustStoredSettings
import com.ekkus.silentdisco.core.rust.RustTrustedDevice

private const val LEGACY_PREFERENCES_NAME = "silent-disco"

data class AndroidStorageSnapshot(
    val schemaVersion: Int,
    val settings: RustStoredSettings,
    val trustedDevices: List<RustTrustedDevice>,
    val legacyImport: RustLegacyImportOutcome,
)

/**
 * Android control-plane owner for one process-lifetime Rust storage session.
 *
 * This class chooses the app-private path and reads the fixed legacy Android
 * key contract. All validation, migrations, SQL, transactions, and durable
 * domain persistence remain Rust-owned. Initialization has no SharedPreferences
 * fallback: any failure closes the candidate Rust worker and is rethrown.
 */
class AndroidStorageRepository(
    private val context: Context,
    private val clock: () -> Long = System::currentTimeMillis,
    private val legacyReader: LegacyAndroidImportReader = LegacyAndroidImportReader(clock),
) : AutoCloseable {
    private var session: RustStorageSession? = null

    @Synchronized
    fun initialize(): AndroidStorageSnapshot {
        session?.let { existing ->
            return readCurrentSnapshot(
                existing,
                RustLegacyImportOutcome(
                    disposition = "already_initialized",
                    importVersion = LEGACY_ANDROID_IMPORT_VERSION,
                    completedAtMs = clock(),
                    settingsImported = existing.loadSettings() != null,
                    trustedDeviceCount = existing.listTrustedDevices().size,
                ),
            )
        }
        val databaseFile = AndroidDatabasePathProvider.resolve(context)
        val candidate = RustStorageBridge.open(databaseFile.absolutePath)
        try {
            val preferences = context.getSharedPreferences(
                LEGACY_PREFERENCES_NAME,
                Context.MODE_PRIVATE,
            )
            val legacySnapshot = legacyReader.read(preferences)
            val importOutcome = candidate.importLegacy(legacySnapshot.request)
            legacyReader.deleteCommittedKeys(preferences, legacySnapshot)
            val persistedSettings = candidate.loadSettings() ?: RustStoredSettings.defaults(clock()).also {
                candidate.saveSettings(it)
            }
            val snapshot = AndroidStorageSnapshot(
                schemaVersion = candidate.schemaVersion,
                settings = persistedSettings,
                trustedDevices = candidate.listTrustedDevices(),
                legacyImport = importOutcome,
            )
            session = candidate
            return snapshot
        } catch (error: Throwable) {
            runCatching(candidate::close).exceptionOrNull()?.let(error::addSuppressed)
            throw error
        }
    }

    @Synchronized
    fun saveSettings(settings: RustStoredSettings): RustStoredSettings =
        requireSession().saveSettings(settings)

    @Synchronized
    fun upsertTrustedDevice(device: RustTrustedDevice): RustTrustedDevice =
        requireSession().upsertTrustedDevice(device)

    @Synchronized
    fun deleteTrustedDevice(deviceId: String): Boolean =
        requireSession().deleteTrustedDevice(deviceId)

    @Synchronized
    fun listTrustedDevices(): List<RustTrustedDevice> =
        requireSession().listTrustedDevices()

    @Synchronized
    override fun close() {
        val current = session ?: return
        current.close()
        session = null
    }

    private fun readCurrentSnapshot(
        current: RustStorageSession,
        importOutcome: RustLegacyImportOutcome,
    ): AndroidStorageSnapshot {
        val settings = current.loadSettings() ?: throw AndroidStorageInitializationException(
            "Rust storage was initialized without the required settings row",
        )
        return AndroidStorageSnapshot(
            schemaVersion = current.schemaVersion,
            settings = settings,
            trustedDevices = current.listTrustedDevices(),
            legacyImport = importOutcome,
        )
    }

    private fun requireSession(): RustStorageSession = session ?: throw AndroidStorageInitializationException(
        "Rust storage has not completed initialization",
    )
}

class AndroidStorageInitializationException(
    message: String,
) : IllegalStateException(message)
