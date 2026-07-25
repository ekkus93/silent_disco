package com.ekkus.silentdisco.core.rust

internal const val SUPPORTED_RUST_CORE_ABI_VERSION: Int = 1

class RustCoreUnavailableException(
    message: String,
    cause: Throwable,
) : IllegalStateException(message, cause)

class UnsupportedRustCoreAbiException(
    val actualVersion: Int,
) : IllegalStateException(
    "Unsupported Rust core ABI version $actualVersion; expected $SUPPORTED_RUST_CORE_ABI_VERSION",
)

internal fun validateRustCoreAbiVersion(version: Int): Int {
    if (version != SUPPORTED_RUST_CORE_ABI_VERSION) {
        throw UnsupportedRustCoreAbiException(version)
    }
    return version
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

    /**
     * Returns the supported native ABI version or throws with the original
     * loading/linkage failure. No synthetic version is returned on failure.
     */
    fun requireSupportedAbiVersion(): Int {
        loadFailure?.let { error ->
            throw RustCoreUnavailableException(
                message = "Unable to load native Rust core library '$LibraryName'",
                cause = error,
            )
        }

        val version = try {
            nativeAbiVersion()
        } catch (error: UnsatisfiedLinkError) {
            throw RustCoreUnavailableException(
                message = "Native Rust core library loaded but its ABI entry point is unavailable",
                cause = error,
            )
        }
        return validateRustCoreAbiVersion(version)
    }
}
