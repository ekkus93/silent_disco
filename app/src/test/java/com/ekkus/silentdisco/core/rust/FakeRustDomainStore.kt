package com.ekkus.silentdisco.core.rust

/**
 * A [RustDomainStore] test double that records every call and can be
 * configured to fail [saveTuning]/[trustDevice] with a real exception, so a
 * test can assert on the real completion/failure fact a storage effect
 * reports back -- not a synthesized success.
 */
class FakeRustDomainStore(
    private val initializeResult: RustStoredTuningSettings = RustStoredTuningSettings(updatedAtMs = 0L),
) : RustDomainStore {
    var saveTuningFailure: Throwable? = null
    var trustDeviceFailure: Throwable? = null

    var initializeCallCount = 0
    val saveTuningCalls = mutableListOf<RustStoredTuningSettings>()
    val trustDeviceCalls = mutableListOf<Triple<String, String, Long>>()
    var closeCallCount = 0

    override suspend fun initialize(): RustStoredTuningSettings {
        initializeCallCount += 1
        return initializeResult
    }

    override suspend fun saveTuning(settings: RustStoredTuningSettings) {
        saveTuningCalls += settings
        saveTuningFailure?.let { throw it }
    }

    override suspend fun trustDevice(deviceId: String, displayName: String, observedAtMs: Long) {
        trustDeviceCalls += Triple(deviceId, displayName, observedAtMs)
        trustDeviceFailure?.let { throw it }
    }

    override suspend fun close() {
        closeCallCount += 1
    }
}
