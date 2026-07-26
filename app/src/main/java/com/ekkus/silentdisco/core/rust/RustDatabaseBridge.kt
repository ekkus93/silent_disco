package com.ekkus.silentdisco.core.rust

private const val DATABASE_SUCCESS = 0
private const val DATABASE_NOT_FOUND = 1
private const val DATABASE_ALREADY_IMPORTED = 2

class RustDatabaseException(
    val statusCode: Int,
    operation: String,
) : IllegalStateException(
    "$operation failed in the Rust database: ${databaseStatusDescription(statusCode)} " +
        "(status=$statusCode)",
)

class RustDatabaseClosedException : IllegalStateException(
    "Rust database handle is already closed",
)

class RustDatabaseBridgeProtocolException(
    message: String,
) : IllegalStateException(message)

data class RustStoredTuningSettings(
    val syncSampleWindow: Int = 12,
    val syncCadenceMs: Long = 2_000,
    val startupBufferMs: Long = 400,
    val latePacketThresholdMs: Long = 40,
    val hardResyncThresholdMs: Long = 120,
    val syncDriftThresholdMs: Double = 18.0,
    val scanWindowMs: Long = 3_000,
    val updatedAtMs: Long,
)

enum class RustLegacyImportOutcome {
    IMPORTED,
    ALREADY_IMPORTED,
}

private fun databaseStatusDescription(statusCode: Int): String = when (statusCode) {
    -100 -> "invalid database argument"
    -101 -> "invalid or already-closed database handle"
    -102 -> "database open or durability configuration failed"
    -103 -> "database migration failed"
    -104 -> "database integrity or worker state failed"
    -105 -> "database is busy"
    -106 -> "database transaction failed"
    -107 -> "database query failed"
    -108 -> "database constraint was violated"
    -109 -> "database close failed"
    -110 -> "database worker is unavailable"
    -111 -> "native database registry lock was poisoned"
    -112 -> "native database handle space was exhausted"
    -113 -> "JNI value conversion failed"
    -114 -> "cached settings are unavailable"
    else -> "unknown native database status"
}

private fun requireDatabaseSuccess(
    statusCode: Int,
    operation: String,
) {
    if (statusCode != DATABASE_SUCCESS) {
        throw RustDatabaseException(statusCode, operation)
    }
}

class RustDatabase internal constructor(
    nativeHandle: Long,
) : AutoCloseable {
    private var handle: Long = nativeHandle

    @Synchronized
    fun importLegacy(
        settings: RustStoredTuningSettings,
        trustedDeviceIds: Array<String>,
    ): RustLegacyImportOutcome {
        val status = RustDatabaseBridge.importLegacy(
            handle = requireOpenHandle(),
            settings = settings,
            trustedDeviceIds = trustedDeviceIds,
        )
        return when (status) {
            DATABASE_SUCCESS -> RustLegacyImportOutcome.IMPORTED
            DATABASE_ALREADY_IMPORTED -> RustLegacyImportOutcome.ALREADY_IMPORTED
            else -> throw RustDatabaseException(status, "import legacy Android persistence")
        }
    }

    @Synchronized
    fun loadSettings(): RustStoredTuningSettings? {
        val currentHandle = requireOpenHandle()
        return when (val status = RustDatabaseBridge.loadSettingsStatus(currentHandle)) {
            DATABASE_SUCCESS -> RustStoredTuningSettings(
                syncSampleWindow = requireNonNegative(
                    "sync sample window",
                    RustDatabaseBridge.cachedSyncSampleWindow(currentHandle),
                ),
                syncCadenceMs = requireNonNegative(
                    "sync cadence",
                    RustDatabaseBridge.cachedSyncCadenceMs(currentHandle),
                ),
                startupBufferMs = requireNonNegative(
                    "startup buffer",
                    RustDatabaseBridge.cachedStartupBufferMs(currentHandle),
                ),
                latePacketThresholdMs = requireNonNegative(
                    "late packet threshold",
                    RustDatabaseBridge.cachedLatePacketThresholdMs(currentHandle),
                ),
                hardResyncThresholdMs = requireNonNegative(
                    "hard resync threshold",
                    RustDatabaseBridge.cachedHardResyncThresholdMs(currentHandle),
                ),
                syncDriftThresholdMs = decodeFiniteDatabaseDouble(
                    "sync drift threshold",
                    RustDatabaseBridge.cachedSyncDriftThresholdBits(currentHandle),
                ),
                scanWindowMs = requireNonNegative(
                    "scan window",
                    RustDatabaseBridge.cachedScanWindowMs(currentHandle),
                ),
                updatedAtMs = requireNonNegative(
                    "settings update time",
                    RustDatabaseBridge.cachedUpdatedAtMs(currentHandle),
                ),
            )
            DATABASE_NOT_FOUND -> null
            else -> throw RustDatabaseException(status, "load persisted settings")
        }
    }

    @Synchronized
    fun saveSettings(settings: RustStoredTuningSettings) {
        requireDatabaseSuccess(
            RustDatabaseBridge.saveSettings(requireOpenHandle(), settings),
            "save persisted settings",
        )
    }

    @Synchronized
    fun trustDevice(
        deviceId: String,
        displayName: String,
        observedAtMs: Long,
    ) {
        requireDatabaseSuccess(
            RustDatabaseBridge.upsertTrusted(
                handle = requireOpenHandle(),
                deviceId = deviceId,
                displayName = displayName,
                observedAtMs = observedAtMs,
            ),
            "persist trusted device",
        )
    }

    @Synchronized
    fun isTrusted(deviceId: String): Boolean {
        val status = RustDatabaseBridge.isTrusted(requireOpenHandle(), deviceId)
        return when (status) {
            1 -> true
            0 -> false
            else -> throw RustDatabaseException(status, "query trusted device")
        }
    }

    @Synchronized
    override fun close() {
        val currentHandle = requireOpenHandle()
        requireDatabaseSuccess(
            RustDatabaseBridge.close(currentHandle),
            "checkpoint and close database",
        )
        handle = 0L
    }

    private fun requireOpenHandle(): Long {
        if (handle <= 0L) {
            throw RustDatabaseClosedException()
        }
        return handle
    }
}

internal fun decodeFiniteDatabaseDouble(
    fieldName: String,
    bits: Long,
): Double {
    val value = Double.fromBits(bits)
    if (!value.isFinite()) {
        throw RustDatabaseBridgeProtocolException(
            "Rust database returned non-finite $fieldName",
        )
    }
    return value
}

private fun requireNonNegative(
    fieldName: String,
    value: Int,
): Int {
    if (value < 0) {
        throw RustDatabaseBridgeProtocolException(
            "Rust database returned negative $fieldName",
        )
    }
    return value
}

private fun requireNonNegative(
    fieldName: String,
    value: Long,
): Long {
    if (value < 0L) {
        throw RustDatabaseBridgeProtocolException(
            "Rust database returned negative $fieldName",
        )
    }
    return value
}

object RustDatabaseBridge {
    private external fun nativeDatabaseOpen(path: String): Long

    private external fun nativeDatabaseClose(handle: Long): Int

    private external fun nativeDatabaseImportLegacy(
        handle: Long,
        version: Int,
        syncSampleWindow: Int,
        syncCadenceMs: Long,
        startupBufferMs: Long,
        latePacketThresholdMs: Long,
        hardResyncThresholdMs: Long,
        syncDriftThresholdMs: Double,
        scanWindowMs: Long,
        trustedDeviceIds: Array<String>,
        importedAtMs: Long,
    ): Int

    private external fun nativeDatabaseLoadSettingsStatus(handle: Long): Int

    private external fun nativeDatabaseCachedSyncSampleWindow(handle: Long): Int

    private external fun nativeDatabaseCachedSyncCadenceMs(handle: Long): Long

    private external fun nativeDatabaseCachedStartupBufferMs(handle: Long): Long

    private external fun nativeDatabaseCachedLatePacketThresholdMs(handle: Long): Long

    private external fun nativeDatabaseCachedHardResyncThresholdMs(handle: Long): Long

    private external fun nativeDatabaseCachedSyncDriftThresholdBits(handle: Long): Long

    private external fun nativeDatabaseCachedScanWindowMs(handle: Long): Long

    private external fun nativeDatabaseCachedUpdatedAtMs(handle: Long): Long

    private external fun nativeDatabaseSaveSettings(
        handle: Long,
        syncSampleWindow: Int,
        syncCadenceMs: Long,
        startupBufferMs: Long,
        latePacketThresholdMs: Long,
        hardResyncThresholdMs: Long,
        syncDriftThresholdMs: Double,
        scanWindowMs: Long,
        updatedAtMs: Long,
    ): Int

    private external fun nativeDatabaseUpsertTrusted(
        handle: Long,
        deviceId: String,
        displayName: String,
        observedAtMs: Long,
    ): Int

    private external fun nativeDatabaseIsTrusted(
        handle: Long,
        deviceId: String,
    ): Int

    fun open(path: String): RustDatabase {
        RustCoreBridge.requireSupportedAbiVersion()
        val handle = invokeNative("open the Rust database") {
            nativeDatabaseOpen(path)
        }
        if (handle <= 0L) {
            throw RustDatabaseException(handle.toInt(), "open database")
        }
        return RustDatabase(handle)
    }

    internal fun importLegacy(
        handle: Long,
        settings: RustStoredTuningSettings,
        trustedDeviceIds: Array<String>,
    ): Int = invokeNative("import legacy Android persistence") {
        nativeDatabaseImportLegacy(
            handle,
            1,
            settings.syncSampleWindow,
            settings.syncCadenceMs,
            settings.startupBufferMs,
            settings.latePacketThresholdMs,
            settings.hardResyncThresholdMs,
            settings.syncDriftThresholdMs,
            settings.scanWindowMs,
            trustedDeviceIds,
            settings.updatedAtMs,
        )
    }

    internal fun loadSettingsStatus(handle: Long): Int =
        invokeNative("load persisted settings") { nativeDatabaseLoadSettingsStatus(handle) }

    internal fun cachedSyncSampleWindow(handle: Long): Int =
        invokeNative("read persisted sync sample window") {
            nativeDatabaseCachedSyncSampleWindow(handle)
        }

    internal fun cachedSyncCadenceMs(handle: Long): Long =
        invokeNative("read persisted sync cadence") { nativeDatabaseCachedSyncCadenceMs(handle) }

    internal fun cachedStartupBufferMs(handle: Long): Long =
        invokeNative("read persisted startup buffer") {
            nativeDatabaseCachedStartupBufferMs(handle)
        }

    internal fun cachedLatePacketThresholdMs(handle: Long): Long =
        invokeNative("read persisted late packet threshold") {
            nativeDatabaseCachedLatePacketThresholdMs(handle)
        }

    internal fun cachedHardResyncThresholdMs(handle: Long): Long =
        invokeNative("read persisted hard resync threshold") {
            nativeDatabaseCachedHardResyncThresholdMs(handle)
        }

    internal fun cachedSyncDriftThresholdBits(handle: Long): Long =
        invokeNative("read persisted sync drift threshold") {
            nativeDatabaseCachedSyncDriftThresholdBits(handle)
        }

    internal fun cachedScanWindowMs(handle: Long): Long =
        invokeNative("read persisted scan window") { nativeDatabaseCachedScanWindowMs(handle) }

    internal fun cachedUpdatedAtMs(handle: Long): Long =
        invokeNative("read persisted settings timestamp") { nativeDatabaseCachedUpdatedAtMs(handle) }

    internal fun saveSettings(
        handle: Long,
        settings: RustStoredTuningSettings,
    ): Int = invokeNative("save persisted settings") {
        nativeDatabaseSaveSettings(
            handle,
            settings.syncSampleWindow,
            settings.syncCadenceMs,
            settings.startupBufferMs,
            settings.latePacketThresholdMs,
            settings.hardResyncThresholdMs,
            settings.syncDriftThresholdMs,
            settings.scanWindowMs,
            settings.updatedAtMs,
        )
    }

    internal fun upsertTrusted(
        handle: Long,
        deviceId: String,
        displayName: String,
        observedAtMs: Long,
    ): Int = invokeNative("persist trusted device") {
        nativeDatabaseUpsertTrusted(handle, deviceId, displayName, observedAtMs)
    }

    internal fun isTrusted(
        handle: Long,
        deviceId: String,
    ): Int = invokeNative("query trusted device") {
        nativeDatabaseIsTrusted(handle, deviceId)
    }

    internal fun close(handle: Long): Int =
        invokeNative("checkpoint and close database") { nativeDatabaseClose(handle) }

    private inline fun <T> invokeNative(
        operation: String,
        call: () -> T,
    ): T = try {
        call()
    } catch (error: UnsatisfiedLinkError) {
        throw RustCoreUnavailableException(
            message = "Native Rust core library loaded but cannot $operation",
            cause = error,
        )
    }
}
