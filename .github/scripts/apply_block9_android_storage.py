from pathlib import Path


def write(path: str, content: str) -> None:
    file_path = Path(path)
    file_path.parent.mkdir(parents=True, exist_ok=True)
    file_path.write_text(content, encoding="utf-8")


def replace_exact(path: str, old: str, new: str) -> None:
    file_path = Path(path)
    text = file_path.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{path}: expected one match, found {count}: {old[:100]!r}")
    file_path.write_text(text.replace(old, new), encoding="utf-8")


write(
    "app/src/main/java/com/ekkus/silentdisco/core/rust/RustStorageBridge.kt",
    r'''package com.ekkus.silentdisco.core.rust

import kotlinx.serialization.KSerializer
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerializationException
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json

private val storageJson = Json {
    encodeDefaults = true
    explicitNulls = true
    ignoreUnknownKeys = false
}

@Serializable
internal data class RustStorageEnvelope<T>(
    val ok: Boolean,
    val result: T? = null,
    val error: RustStorageErrorDto? = null,
)

@Serializable
internal data class RustStorageErrorDto(
    val code: String,
    val operation: String,
    val message: String,
    val retryable: Boolean,
    val coreRemainsUsable: Boolean,
    val schemaVersion: Int? = null,
)

class RustStorageException internal constructor(
    val code: String,
    val operation: String,
    override val message: String,
    val retryable: Boolean,
    val coreRemainsUsable: Boolean,
    val schemaVersion: Int?,
) : IllegalStateException("$operation failed: $message (code=$code)")

class RustStorageBridgeProtocolException(
    message: String,
    cause: Throwable? = null,
) : IllegalStateException(message, cause)

class RustStorageSessionClosedException : IllegalStateException(
    "Rust storage session is already closed",
)

@Serializable
data class RustStoredSettings(
    val syncSampleWindow: Int,
    val syncCadenceMs: Long,
    val startupBufferMs: Long,
    val latePacketThresholdMs: Long,
    val hardResyncThresholdMs: Long,
    val syncDriftThresholdMs: Double,
    val scanWindowMs: Long,
    val updatedAtMs: Long,
) {
    companion object {
        fun defaults(updatedAtMs: Long): RustStoredSettings = RustStoredSettings(
            syncSampleWindow = 12,
            syncCadenceMs = 2_000L,
            startupBufferMs = 400L,
            latePacketThresholdMs = 40L,
            hardResyncThresholdMs = 120L,
            syncDriftThresholdMs = 18.0,
            scanWindowMs = 3_000L,
            updatedAtMs = updatedAtMs,
        )
    }
}

@Serializable
data class RustTrustedDevice(
    val deviceId: String,
    val displayName: String,
    val publicKey: List<Int>? = null,
    val privateKeyRef: String? = null,
    val trustState: String,
    val firstSeenMs: Long,
    val lastSeenMs: Long,
    val updatedAtMs: Long,
)

@Serializable
data class RustLegacyAndroidImport(
    val version: Int = LEGACY_ANDROID_IMPORT_VERSION,
    val importedAtMs: Long,
    val settings: RustStoredSettings?,
    val trustedDevices: List<RustTrustedDevice>,
)

@Serializable
data class RustLegacyImportOutcome(
    val disposition: String,
    val importVersion: Int,
    val completedAtMs: Long,
    val settingsImported: Boolean,
    val trustedDeviceCount: Int,
)

@Serializable
private data class RustStorageOpenResult(
    val handle: Long,
    val schemaVersion: Int,
)

@Serializable
private data class DeleteTrustedDeviceRequest(
    val deviceId: String,
)

@Serializable
private data class DeleteTrustedDeviceResult(
    val deleted: Boolean,
)

@Serializable
private data class CloseStorageResult(
    val closed: Boolean,
)

const val LEGACY_ANDROID_IMPORT_VERSION: Int = 1

internal fun <T> decodeRustStorageEnvelope(
    rawResponse: String?,
    resultSerializer: KSerializer<T>,
): T? {
    val response = rawResponse ?: throw RustStorageBridgeProtocolException(
        "Rust storage bridge returned a null JNI string",
    )
    val envelope = try {
        storageJson.decodeFromString(
            RustStorageEnvelope.serializer(resultSerializer),
            response,
        )
    } catch (error: SerializationException) {
        throw RustStorageBridgeProtocolException(
            "Rust storage bridge returned invalid or incompatible JSON",
            error,
        )
    }
    if (envelope.ok) {
        if (envelope.error != null) {
            throw RustStorageBridgeProtocolException(
                "Rust storage success response unexpectedly contained an error",
            )
        }
        return envelope.result
    }
    if (envelope.result != null) {
        throw RustStorageBridgeProtocolException(
            "Rust storage failure response unexpectedly contained a result",
        )
    }
    val error = envelope.error ?: throw RustStorageBridgeProtocolException(
        "Rust storage failure response omitted its structured error",
    )
    throw RustStorageException(
        code = error.code,
        operation = error.operation,
        message = error.message,
        retryable = error.retryable,
        coreRemainsUsable = error.coreRemainsUsable,
        schemaVersion = error.schemaVersion,
    )
}

private fun <T> requireRustStorageResult(
    rawResponse: String?,
    resultSerializer: KSerializer<T>,
    operation: String,
): T = decodeRustStorageEnvelope(rawResponse, resultSerializer)
    ?: throw RustStorageBridgeProtocolException(
        "Rust storage $operation response omitted its result",
    )

private fun <T> encodeRustStorageRequest(
    value: T,
    serializer: KSerializer<T>,
): String = try {
    storageJson.encodeToString(serializer, value)
} catch (error: SerializationException) {
    throw RustStorageBridgeProtocolException(
        "Android could not serialize a Rust storage request",
        error,
    )
}

private inline fun invokeStorageNative(
    operation: String,
    call: () -> String?,
): String? = try {
    call()
} catch (error: UnsatisfiedLinkError) {
    throw RustCoreUnavailableException(
        message = "Native Rust core library loaded but cannot $operation",
        cause = error,
    )
} catch (error: SecurityException) {
    throw RustCoreUnavailableException(
        message = "Android security policy prevented the native Rust core from trying to $operation",
        cause = error,
    )
}

class RustStorageSession internal constructor(
    nativeHandle: Long,
    val schemaVersion: Int,
) : AutoCloseable {
    private var handle: Long = nativeHandle

    @Synchronized
    fun importLegacy(import: RustLegacyAndroidImport): RustLegacyImportOutcome {
        val currentHandle = requireOpenHandle()
        val request = encodeRustStorageRequest(import, RustLegacyAndroidImport.serializer())
        return requireRustStorageResult(
            invokeStorageNative("import legacy Android data") {
                RustStorageNativeBridge.nativeStorageImportLegacy(currentHandle, request)
            },
            RustLegacyImportOutcome.serializer(),
            "legacy import",
        )
    }

    @Synchronized
    fun loadSettings(): RustStoredSettings? = decodeRustStorageEnvelope(
        invokeStorageNative("load settings") {
            RustStorageNativeBridge.nativeStorageLoadSettings(requireOpenHandle())
        },
        RustStoredSettings.serializer(),
    )

    @Synchronized
    fun saveSettings(settings: RustStoredSettings): RustStoredSettings {
        val currentHandle = requireOpenHandle()
        val request = encodeRustStorageRequest(settings, RustStoredSettings.serializer())
        return requireRustStorageResult(
            invokeStorageNative("save settings") {
                RustStorageNativeBridge.nativeStorageSaveSettings(currentHandle, request)
            },
            RustStoredSettings.serializer(),
            "save settings",
        )
    }

    @Synchronized
    fun listTrustedDevices(): List<RustTrustedDevice> = requireRustStorageResult(
        invokeStorageNative("list trusted devices") {
            RustStorageNativeBridge.nativeStorageListTrustedDevices(requireOpenHandle())
        },
        ListSerializer(RustTrustedDevice.serializer()),
        "trusted-device list",
    )

    @Synchronized
    fun upsertTrustedDevice(device: RustTrustedDevice): RustTrustedDevice {
        val currentHandle = requireOpenHandle()
        val request = encodeRustStorageRequest(device, RustTrustedDevice.serializer())
        return requireRustStorageResult(
            invokeStorageNative("save a trusted device") {
                RustStorageNativeBridge.nativeStorageUpsertTrustedDevice(currentHandle, request)
            },
            RustTrustedDevice.serializer(),
            "trusted-device save",
        )
    }

    @Synchronized
    fun deleteTrustedDevice(deviceId: String): Boolean {
        val currentHandle = requireOpenHandle()
        val request = encodeRustStorageRequest(
            DeleteTrustedDeviceRequest(deviceId),
            DeleteTrustedDeviceRequest.serializer(),
        )
        return requireRustStorageResult(
            invokeStorageNative("delete a trusted device") {
                RustStorageNativeBridge.nativeStorageDeleteTrustedDevice(currentHandle, request)
            },
            DeleteTrustedDeviceResult.serializer(),
            "trusted-device delete",
        ).deleted
    }

    @Synchronized
    override fun close() {
        val currentHandle = requireOpenHandle()
        val result = requireRustStorageResult(
            invokeStorageNative("close storage") {
                RustStorageNativeBridge.nativeStorageClose(currentHandle)
            },
            CloseStorageResult.serializer(),
            "close",
        )
        if (!result.closed) {
            throw RustStorageBridgeProtocolException(
                "Rust storage close response did not confirm closure",
            )
        }
        handle = 0L
    }

    private fun requireOpenHandle(): Long {
        if (handle <= 0L) throw RustStorageSessionClosedException()
        return handle
    }
}

object RustStorageBridge {
    fun open(databasePath: String): RustStorageSession {
        require(databasePath.isNotBlank()) { "databasePath must not be blank" }
        RustCoreBridge.requireSupportedAbiVersion()
        val result = requireRustStorageResult(
            invokeStorageNative("open storage") {
                RustStorageNativeBridge.nativeStorageOpen(databasePath)
            },
            RustStorageOpenResult.serializer(),
            "open",
        )
        if (result.handle <= 0L) {
            throw RustStorageBridgeProtocolException(
                "Rust storage open response returned a non-positive handle",
            )
        }
        if (result.schemaVersion <= 0) {
            throw RustStorageBridgeProtocolException(
                "Rust storage open response returned an invalid schema version",
            )
        }
        return RustStorageSession(result.handle, result.schemaVersion)
    }
}

internal object RustStorageNativeBridge {
    external fun nativeStorageOpen(databasePath: String): String?
    external fun nativeStorageImportLegacy(handle: Long, requestJson: String): String?
    external fun nativeStorageLoadSettings(handle: Long): String?
    external fun nativeStorageSaveSettings(handle: Long, requestJson: String): String?
    external fun nativeStorageListTrustedDevices(handle: Long): String?
    external fun nativeStorageUpsertTrustedDevice(handle: Long, requestJson: String): String?
    external fun nativeStorageDeleteTrustedDevice(handle: Long, requestJson: String): String?
    external fun nativeStorageClose(handle: Long): String?
}
''',
)

write(
    "app/src/main/java/com/ekkus/silentdisco/core/persistence/AndroidDatabasePathProvider.kt",
    r'''package com.ekkus.silentdisco.core.persistence

import android.content.Context
import java.io.File

private const val DATABASE_DIRECTORY_NAME = "databases"
private const val DATABASE_FILE_NAME = "silent-disco.sqlite3"

/**
 * Selects the app-private Rust database path under Android's no-backup root.
 *
 * Kotlin creates and validates only the parent directory. It never creates,
 * opens, inspects, migrates, or repairs the database file; only Rust may do so.
 * Keeping the database under [Context.getNoBackupFilesDir] intentionally
 * excludes authoritative session, settings, and trust data from Android backup.
 */
object AndroidDatabasePathProvider {
    fun resolve(context: Context): File = resolveNoBackupPath(context.noBackupFilesDir)

    internal fun resolveNoBackupPath(noBackupRoot: File): File {
        val root = noBackupRoot.absoluteFile
        if (!root.exists() || !root.isDirectory) {
            throw AndroidDatabasePathException(
                "Android no-backup root is unavailable or is not a directory: ${root.path}",
            )
        }
        val parent = File(root, DATABASE_DIRECTORY_NAME)
        if (parent.exists()) {
            if (!parent.isDirectory) {
                throw AndroidDatabasePathException(
                    "Rust database parent exists but is not a directory: ${parent.path}",
                )
            }
        } else if (!parent.mkdirs()) {
            throw AndroidDatabasePathException(
                "Unable to create Rust database parent directory: ${parent.path}",
            )
        }
        val canonicalRoot = root.canonicalFile
        val canonicalParent = parent.canonicalFile
        if (canonicalParent.parentFile != canonicalRoot) {
            throw AndroidDatabasePathException(
                "Rust database parent escaped the Android no-backup root",
            )
        }
        return File(canonicalParent, DATABASE_FILE_NAME).absoluteFile
    }
}

class AndroidDatabasePathException(
    message: String,
    cause: Throwable? = null,
) : IllegalStateException(message, cause)
''',
)

write(
    "app/src/main/java/com/ekkus/silentdisco/core/persistence/LegacyAndroidImportReader.kt",
    r'''package com.ekkus.silentdisco.core.persistence

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
''',
)

write(
    "app/src/main/java/com/ekkus/silentdisco/core/persistence/AndroidStorageRepository.kt",
    r'''package com.ekkus.silentdisco.core.persistence

import android.content.Context
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
''',
)

write(
    "app/src/test/java/com/ekkus/silentdisco/core/persistence/AndroidDatabasePathProviderTest.kt",
    r'''package com.ekkus.silentdisco.core.persistence

import java.nio.file.Files
import kotlin.io.path.deleteRecursively
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertFailsWith
import kotlin.test.assertTrue

class AndroidDatabasePathProviderTest {
    @Test
    fun createsOnlyTheNoBackupParentAndReturnsTheCompleteDatabasePath() {
        val root = Files.createTempDirectory("silent-disco-no-backup")
        try {
            val database = AndroidDatabasePathProvider.resolveNoBackupPath(root.toFile())

            assertEquals(root.resolve("databases/silent-disco.sqlite3").toFile(), database)
            assertTrue(root.resolve("databases").toFile().isDirectory)
            assertFalse(database.exists(), "Kotlin must not create or open the database file")
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun rejectsAParentPathOccupiedByARegularFile() {
        val root = Files.createTempDirectory("silent-disco-no-backup-file")
        try {
            Files.writeString(root.resolve("databases"), "not a directory")
            assertFailsWith<AndroidDatabasePathException> {
                AndroidDatabasePathProvider.resolveNoBackupPath(root.toFile())
            }
        } finally {
            root.deleteRecursively()
        }
    }
}
''',
)

write(
    "app/src/test/java/com/ekkus/silentdisco/core/persistence/LegacyAndroidImportReaderTest.kt",
    r'''package com.ekkus.silentdisco.core.persistence

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertNull

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

        assertEquals(16, snapshot.request.settings?.syncSampleWindow)
        assertEquals(2_500L, snapshot.request.settings?.syncCadenceMs)
        assertEquals(24.0, snapshot.request.settings?.syncDriftThresholdMs)
        assertEquals(3_000L, snapshot.request.settings?.scanWindowMs)
        assertEquals(listOf("listener-a"), snapshot.request.trustedDevices.map { it.deviceId })
        assertEquals(
            setOf(
                LegacyPreferencesContract.SYNC_SAMPLE_WINDOW,
                LegacyPreferencesContract.SYNC_CADENCE_MS,
                LegacyPreferencesContract.SYNC_DRIFT_THRESHOLD_BITS,
                "trusted:listener-a",
                "trusted:listener-b",
            ),
            snapshot.keysToDeleteAfterCommit,
        )
    }

    @Test
    fun emptyLegacyMapStillProducesAnExplicitVersionedImport() {
        val snapshot = buildLegacyAndroidImportSnapshot(emptyMap(), importedAtMs = 1L)

        assertNull(snapshot.request.settings)
        assertEquals(emptyList(), snapshot.request.trustedDevices)
        assertEquals(emptySet(), snapshot.keysToDeleteAfterCommit)
    }

    @Test
    fun rejectsLegacyTypeMismatchWithoutProducingCleanupKeys() {
        assertFailsWith<LegacyPreferencesReadException> {
            buildLegacyAndroidImportSnapshot(
                mapOf(LegacyPreferencesContract.SYNC_CADENCE_MS to 2_000),
                importedAtMs = 1L,
            )
        }
        assertFailsWith<LegacyPreferencesReadException> {
            buildLegacyAndroidImportSnapshot(
                mapOf("trusted:listener-a" to "true"),
                importedAtMs = 1L,
            )
        }
    }
}
''',
)

write(
    "app/src/test/java/com/ekkus/silentdisco/core/rust/RustStorageBridgeJsonTest.kt",
    r'''package com.ekkus.silentdisco.core.rust

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertNull

class RustStorageBridgeJsonTest {
    @Test
    fun decodesSuccessfulAndNullableResults() {
        val settings = decodeRustStorageEnvelope(
            """{"ok":true,"result":{"syncSampleWindow":12,"syncCadenceMs":2000,"startupBufferMs":400,"latePacketThresholdMs":40,"hardResyncThresholdMs":120,"syncDriftThresholdMs":18.0,"scanWindowMs":3000,"updatedAtMs":100},"error":null}""",
            RustStoredSettings.serializer(),
        )
        assertEquals(2_000L, settings?.syncCadenceMs)

        assertNull(
            decodeRustStorageEnvelope(
                """{"ok":true,"result":null,"error":null}""",
                RustStoredSettings.serializer(),
            ),
        )
    }

    @Test
    fun surfacesStructuredRustFailureWithoutConvertingItToDefaults() {
        val error = assertFailsWith<RustStorageException> {
            decodeRustStorageEnvelope(
                """{"ok":false,"result":null,"error":{"code":"storage_corruption","operation":"open","message":"integrity check failed","retryable":false,"coreRemainsUsable":false,"schemaVersion":2}}""",
                RustStoredSettings.serializer(),
            )
        }
        assertEquals("storage_corruption", error.code)
        assertEquals("open", error.operation)
        assertEquals(2, error.schemaVersion)
        assertEquals(false, error.coreRemainsUsable)
    }

    @Test
    fun rejectsUnknownProtocolFields() {
        assertFailsWith<RustStorageBridgeProtocolException> {
            decodeRustStorageEnvelope(
                """{"ok":true,"result":null,"error":null,"unexpected":true}""",
                RustStoredSettings.serializer(),
            )
        }
    }
}
''',
)

replace_exact(
    "app/src/main/java/com/ekkus/silentdisco/app/SilentDiscoApplication.kt",
    '''package com.ekkus.silentdisco.app

import android.app.Application

class SilentDiscoApplication : Application()
''',
    '''package com.ekkus.silentdisco.app

import android.app.Application
import com.ekkus.silentdisco.core.persistence.AndroidStorageRepository

class SilentDiscoApplication : Application() {
    val storageRepository: AndroidStorageRepository by lazy(LazyThreadSafetyMode.SYNCHRONIZED) {
        AndroidStorageRepository(this)
    }
}
''',
)
