package com.ekkus.silentdisco.core.rust

internal const val SUPPORTED_RUST_CORE_ABI_VERSION: Int = 1

private const val NATIVE_SYNC_ACCEPTED: Int = 0
private const val NATIVE_SYNC_REJECTED: Int = 1

class RustCoreUnavailableException(
    message: String,
    cause: Throwable,
) : IllegalStateException(message, cause)

class UnsupportedRustCoreAbiException(
    val actualVersion: Int,
) : IllegalStateException(
    "Unsupported Rust core ABI version $actualVersion; expected $SUPPORTED_RUST_CORE_ABI_VERSION",
)

class RustSyncNativeException(
    val statusCode: Int,
    operation: String,
) : IllegalStateException(
    "$operation failed in the Rust synchronization core: ${nativeSyncStatusDescription(statusCode)} " +
        "(status=$statusCode)",
)

class RustSyncBridgeProtocolException(
    message: String,
) : IllegalStateException(message)

class RustSyncEstimatorClosedException : IllegalStateException(
    "Rust synchronization estimator is already closed",
)

enum class RustSyncConfidence(
    val stableCode: Int,
) {
    UNKNOWN(1),
    POOR(2),
    FAIR(3),
    GOOD(4),
    EXCELLENT(5),
    ;

    companion object {
        internal fun fromStableCode(code: Int): RustSyncConfidence = entries.firstOrNull {
            it.stableCode == code
        } ?: throw RustSyncBridgeProtocolException(
            "Rust synchronization core returned unsupported confidence code $code",
        )
    }
}

data class RustSyncSnapshot(
    val offsetMs: Double,
    val roundTripTimeMs: Double,
    val jitterMs: Double,
    val skewPpm: Double,
    val confidence: RustSyncConfidence,
    val acceptedSampleCount: Int,
)

data class RustSyncObservation(
    val roundTripTimeMs: Double,
    val offsetMs: Double,
    val accepted: Boolean,
    val acquisitionRejectedSampleCount: Long,
    val acquisitionElapsedMs: Long,
    val acquisitionRttLimitMs: Double,
    val degradedLock: Boolean,
    val snapshot: RustSyncSnapshot,
)

internal fun validateRustCoreAbiVersion(version: Int): Int {
    if (version != SUPPORTED_RUST_CORE_ABI_VERSION) {
        throw UnsupportedRustCoreAbiException(version)
    }
    return version
}

internal fun decodeFiniteNativeDouble(
    fieldName: String,
    bits: Long,
): Double {
    val value = Double.fromBits(bits)
    if (!value.isFinite()) {
        throw RustSyncBridgeProtocolException(
            "Rust synchronization core returned non-finite $fieldName",
        )
    }
    return value
}

private fun nativeSyncStatusDescription(statusCode: Int): String = when (statusCode) {
    -1 -> "invalid estimator configuration"
    -2 -> "invalid or already-destroyed estimator handle"
    -3 -> "negative monotonic timestamp"
    -4 -> "local monotonic clock moved backwards"
    -5 -> "host monotonic clock moved backwards"
    -6 -> "host processing exceeded the measured round trip"
    -7 -> "correlation ID space exhausted"
    -8 -> "native estimator registry lock was poisoned"
    -9 -> "native estimator handle space exhausted"
    -10 -> "accepted sample count exceeded the binding field width"
    -11 -> "duplicate correlation ID"
    -12 -> "pending probe limit reached"
    -13 -> "stale correlation ID"
    -14 -> "correlation timestamp mismatch"
    -15 -> "no successful observation is available"
    else -> "unknown native synchronization status"
}

private fun requireNativeSyncSuccess(
    statusCode: Int,
    operation: String,
) {
    if (statusCode != NATIVE_SYNC_ACCEPTED) {
        throw RustSyncNativeException(statusCode, operation)
    }
}

private fun decodeNonNegativeNativeLong(
    fieldName: String,
    value: Long,
): Long {
    if (value < 0L) {
        throw RustSyncNativeException(value.toInt(), "read $fieldName")
    }
    return value
}

private fun decodeNativeBoolean(
    fieldName: String,
    value: Int,
): Boolean = when (value) {
    0 -> false
    1 -> true
    else -> if (value < 0) {
        throw RustSyncNativeException(value, "read $fieldName")
    } else {
        throw RustSyncBridgeProtocolException(
            "Rust synchronization core returned invalid $fieldName value $value",
        )
    }
}

/**
 * A control-plane handle for the Rust-owned synchronization estimator.
 *
 * All methods are synchronized because native observation fields are read via
 * a fixed primitive ABI after an explicit status check. This class must not be
 * used from the real-time audio path.
 */
class RustSyncEstimator internal constructor(
    nativeHandle: Long,
) : AutoCloseable {
    private var handle: Long = nativeHandle

    @Synchronized
    fun observe(
        t1LocalSendMs: Long,
        t2HostReceiveMs: Long,
        t3HostSendMs: Long,
        t4LocalReceiveMs: Long,
    ): RustSyncObservation {
        val currentHandle = requireOpenHandle()
        val status = RustCoreBridge.observeSyncEstimator(
            handle = currentHandle,
            t1LocalSendMs = t1LocalSendMs,
            t2HostReceiveMs = t2HostReceiveMs,
            t3HostSendMs = t3HostSendMs,
            t4LocalReceiveMs = t4LocalReceiveMs,
        )
        val accepted = when (status) {
            NATIVE_SYNC_ACCEPTED -> true
            NATIVE_SYNC_REJECTED -> false
            else -> throw RustSyncNativeException(status, "observe synchronization sample")
        }
        requireNativeSyncSuccess(
            RustCoreBridge.lastSyncObservationStatus(currentHandle),
            "read latest synchronization observation",
        )
        return RustSyncObservation(
            roundTripTimeMs = decodeFiniteNativeDouble(
                fieldName = "observation RTT",
                bits = RustCoreBridge.lastSyncObservationRttBits(currentHandle),
            ),
            offsetMs = decodeFiniteNativeDouble(
                fieldName = "observation offset",
                bits = RustCoreBridge.lastSyncObservationOffsetBits(currentHandle),
            ),
            accepted = accepted,
            acquisitionRejectedSampleCount = decodeNonNegativeNativeLong(
                fieldName = "acquisition rejected sample count",
                value = RustCoreBridge.lastSyncObservationAcquisitionRejectedCount(currentHandle),
            ),
            acquisitionElapsedMs = decodeNonNegativeNativeLong(
                fieldName = "acquisition elapsed time",
                value = RustCoreBridge.lastSyncObservationAcquisitionElapsedMs(currentHandle),
            ),
            acquisitionRttLimitMs = decodeFiniteNativeDouble(
                fieldName = "acquisition RTT limit",
                bits = RustCoreBridge.lastSyncObservationAcquisitionRttLimitBits(currentHandle),
            ),
            degradedLock = decodeNativeBoolean(
                fieldName = "degraded lock",
                value = RustCoreBridge.lastSyncObservationDegradedLock(currentHandle),
            ),
            snapshot = readSnapshot(currentHandle),
        )
    }

    @Synchronized
    fun snapshot(): RustSyncSnapshot = readSnapshot(requireOpenHandle())

    @Synchronized
    override fun close() {
        val currentHandle = requireOpenHandle()
        requireNativeSyncSuccess(
            RustCoreBridge.destroySyncEstimator(currentHandle),
            "destroy synchronization estimator",
        )
        handle = 0L
    }

    private fun requireOpenHandle(): Long {
        if (handle <= 0L) {
            throw RustSyncEstimatorClosedException()
        }
        return handle
    }

    private fun readSnapshot(currentHandle: Long): RustSyncSnapshot {
        requireNativeSyncSuccess(
            RustCoreBridge.syncEstimatorSnapshotStatus(currentHandle),
            "read synchronization snapshot",
        )
        val acceptedSampleCount = RustCoreBridge.syncEstimatorSnapshotAcceptedCount(currentHandle)
        if (acceptedSampleCount < 0) {
            throw RustSyncBridgeProtocolException(
                "Rust synchronization core returned negative accepted sample count " +
                    acceptedSampleCount,
            )
        }
        return RustSyncSnapshot(
            offsetMs = decodeFiniteNativeDouble(
                fieldName = "snapshot offset",
                bits = RustCoreBridge.syncEstimatorSnapshotOffsetBits(currentHandle),
            ),
            roundTripTimeMs = decodeFiniteNativeDouble(
                fieldName = "snapshot RTT",
                bits = RustCoreBridge.syncEstimatorSnapshotRttBits(currentHandle),
            ),
            jitterMs = decodeFiniteNativeDouble(
                fieldName = "snapshot jitter",
                bits = RustCoreBridge.syncEstimatorSnapshotJitterBits(currentHandle),
            ),
            skewPpm = decodeFiniteNativeDouble(
                fieldName = "snapshot skew",
                bits = RustCoreBridge.syncEstimatorSnapshotSkewBits(currentHandle),
            ),
            confidence = RustSyncConfidence.fromStableCode(
                RustCoreBridge.syncEstimatorSnapshotConfidenceCode(currentHandle),
            ),
            acceptedSampleCount = acceptedSampleCount,
        )
    }
}

/**
 * Android-only loader for the shared Rust core.
 *
 * This bridge is intended for startup, diagnostics, and control-plane calls.
 * It must never be called from the real-time audio render path.
 */
object RustCoreBridge {
    private const val LibraryName = "silent_disco_ffi"

    private val loadFailure: Throwable? = try {
        System.loadLibrary(LibraryName)
        null
    } catch (error: UnsatisfiedLinkError) {
        error
    } catch (error: SecurityException) {
        error
    }

    private external fun nativeAbiVersion(): Int

    private external fun nativeSyncEstimatorCreate(
        maxSamples: Int,
        maxAcceptedRttMs: Double,
    ): Long

    private external fun nativeSyncEstimatorObserve(
        handle: Long,
        t1LocalSendMs: Long,
        t2HostReceiveMs: Long,
        t3HostSendMs: Long,
        t4LocalReceiveMs: Long,
    ): Int

    private external fun nativeSyncEstimatorLastStatus(handle: Long): Int

    private external fun nativeSyncEstimatorLastRttBits(handle: Long): Long

    private external fun nativeSyncEstimatorLastOffsetBits(handle: Long): Long

    private external fun nativeSyncEstimatorLastAcquisitionRejectedCount(handle: Long): Long

    private external fun nativeSyncEstimatorLastAcquisitionElapsedMs(handle: Long): Long

    private external fun nativeSyncEstimatorLastAcquisitionRttLimitBits(handle: Long): Long

    private external fun nativeSyncEstimatorLastDegradedLock(handle: Long): Int

    private external fun nativeSyncEstimatorSnapshotStatus(handle: Long): Int

    private external fun nativeSyncEstimatorSnapshotOffsetBits(handle: Long): Long

    private external fun nativeSyncEstimatorSnapshotRttBits(handle: Long): Long

    private external fun nativeSyncEstimatorSnapshotJitterBits(handle: Long): Long

    private external fun nativeSyncEstimatorSnapshotSkewBits(handle: Long): Long

    private external fun nativeSyncEstimatorSnapshotConfidenceCode(handle: Long): Int

    private external fun nativeSyncEstimatorSnapshotAcceptedCount(handle: Long): Int

    private external fun nativeSyncEstimatorDestroy(handle: Long): Int

    /**
     * Returns the supported native ABI version or throws with the original
     * loading/linkage failure. No synthetic version is returned on failure.
     */
    fun requireSupportedAbiVersion(): Int = validateRustCoreAbiVersion(
        invokeNative("read the Rust core ABI version", ::nativeAbiVersion),
    )

    fun openSyncEstimator(
        maxSamples: Int = 12,
        maxAcceptedRttMs: Double = 200.0,
    ): RustSyncEstimator {
        requireSupportedAbiVersion()
        val handle = invokeNative("create a Rust synchronization estimator") {
            nativeSyncEstimatorCreate(maxSamples, maxAcceptedRttMs)
        }
        if (handle <= 0L) {
            val statusCode = handle.toInt()
            throw RustSyncNativeException(statusCode, "create synchronization estimator")
        }
        return RustSyncEstimator(handle)
    }

    internal fun observeSyncEstimator(
        handle: Long,
        t1LocalSendMs: Long,
        t2HostReceiveMs: Long,
        t3HostSendMs: Long,
        t4LocalReceiveMs: Long,
    ): Int = invokeNative("observe a Rust synchronization sample") {
        nativeSyncEstimatorObserve(
            handle,
            t1LocalSendMs,
            t2HostReceiveMs,
            t3HostSendMs,
            t4LocalReceiveMs,
        )
    }

    internal fun lastSyncObservationStatus(handle: Long): Int =
        invokeNative("validate the latest Rust synchronization observation") {
            nativeSyncEstimatorLastStatus(handle)
        }

    internal fun lastSyncObservationRttBits(handle: Long): Long =
        invokeNative("read the latest Rust synchronization RTT") {
            nativeSyncEstimatorLastRttBits(handle)
        }

    internal fun lastSyncObservationOffsetBits(handle: Long): Long =
        invokeNative("read the latest Rust synchronization offset") {
            nativeSyncEstimatorLastOffsetBits(handle)
        }


    internal fun lastSyncObservationAcquisitionRejectedCount(handle: Long): Long =
        invokeNative("read synchronization acquisition rejected count") {
            nativeSyncEstimatorLastAcquisitionRejectedCount(handle)
        }

    internal fun lastSyncObservationAcquisitionElapsedMs(handle: Long): Long =
        invokeNative("read synchronization acquisition elapsed time") {
            nativeSyncEstimatorLastAcquisitionElapsedMs(handle)
        }

    internal fun lastSyncObservationAcquisitionRttLimitBits(handle: Long): Long =
        invokeNative("read synchronization acquisition RTT limit") {
            nativeSyncEstimatorLastAcquisitionRttLimitBits(handle)
        }

    internal fun lastSyncObservationDegradedLock(handle: Long): Int =
        invokeNative("read synchronization degraded-lock flag") {
            nativeSyncEstimatorLastDegradedLock(handle)
        }

    internal fun syncEstimatorSnapshotStatus(handle: Long): Int =
        invokeNative("validate the Rust synchronization snapshot") {
            nativeSyncEstimatorSnapshotStatus(handle)
        }

    internal fun syncEstimatorSnapshotOffsetBits(handle: Long): Long =
        invokeNative("read the Rust synchronization snapshot offset") {
            nativeSyncEstimatorSnapshotOffsetBits(handle)
        }

    internal fun syncEstimatorSnapshotRttBits(handle: Long): Long =
        invokeNative("read the Rust synchronization snapshot RTT") {
            nativeSyncEstimatorSnapshotRttBits(handle)
        }

    internal fun syncEstimatorSnapshotJitterBits(handle: Long): Long =
        invokeNative("read the Rust synchronization snapshot jitter") {
            nativeSyncEstimatorSnapshotJitterBits(handle)
        }

    internal fun syncEstimatorSnapshotSkewBits(handle: Long): Long =
        invokeNative("read the Rust synchronization snapshot skew") {
            nativeSyncEstimatorSnapshotSkewBits(handle)
        }

    internal fun syncEstimatorSnapshotConfidenceCode(handle: Long): Int =
        invokeNative("read the Rust synchronization confidence") {
            nativeSyncEstimatorSnapshotConfidenceCode(handle)
        }

    internal fun syncEstimatorSnapshotAcceptedCount(handle: Long): Int =
        invokeNative("read the Rust synchronization sample count") {
            nativeSyncEstimatorSnapshotAcceptedCount(handle)
        }

    internal fun destroySyncEstimator(handle: Long): Int =
        invokeNative("destroy the Rust synchronization estimator") {
            nativeSyncEstimatorDestroy(handle)
        }

    private inline fun <T> invokeNative(
        operation: String,
        call: () -> T,
    ): T {
        loadFailure?.let { error ->
            throw RustCoreUnavailableException(
                message = "Unable to load native Rust core library '$LibraryName' while trying to " +
                    operation,
                cause = error,
            )
        }
        return try {
            call()
        } catch (error: UnsatisfiedLinkError) {
            throw RustCoreUnavailableException(
                message = "Native Rust core library loaded but cannot $operation",
                cause = error,
            )
        }
    }
}
