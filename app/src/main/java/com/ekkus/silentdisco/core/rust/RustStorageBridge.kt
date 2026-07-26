package com.ekkus.silentdisco.core.rust

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
