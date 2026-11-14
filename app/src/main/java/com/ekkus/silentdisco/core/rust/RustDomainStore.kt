package com.ekkus.silentdisco.core.rust

/**
 * Android-facing command port for the Rust-owned domain database used by
 * `MainViewModel`'s settings/trusted-device storage effects
 * (`executeRustStorageEffect` in `MainViewModelRustHost.kt`).
 *
 * Exists so that effect-runner path can be exercised in JVM unit tests
 * against a recording fake, without constructing a real
 * `AndroidRustDomainStore` -- which requires a real Android `Context` and
 * opens the native Rust database worker, neither of which is available in a
 * plain JVM test.
 */
interface RustDomainStore {
    suspend fun initialize(): RustStoredTuningSettings

    suspend fun saveTuning(settings: RustStoredTuningSettings)

    suspend fun trustDevice(
        deviceId: String,
        displayName: String,
        observedAtMs: Long = System.currentTimeMillis(),
    )

    /**
     * Runs [load] with the open Rust database handle, so a Rust FFI object can
     * read host-owned records (verified listener trust) without them passing
     * through Kotlin. Throws if the database is not open.
     */
    suspend fun <T> withDatabaseHandle(load: (databaseHandle: Long) -> T): T

    suspend fun close()
}
