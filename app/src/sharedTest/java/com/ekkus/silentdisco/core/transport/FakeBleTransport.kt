package com.ekkus.silentdisco.core.transport

import com.ekkus.silentdisco.core.model.SessionInfo
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * A [BleTransport] test double that records every call and returns a
 * configurable [BleOperationResult], so a test can drive a genuine BLE
 * failure path (distinct from a Wi-Fi Direct failure) through
 * `MainViewModel`'s effect runner without a real `BluetoothManager`.
 */
class FakeBleTransport : BleTransport {
    private val _discoveredSessions = MutableStateFlow<List<SessionInfo>>(emptyList())
    override val discoveredSessions: StateFlow<List<SessionInfo>> = _discoveredSessions.asStateFlow()

    private val _failures = MutableSharedFlow<BleOperationFailure>(extraBufferCapacity = 8)
    override val failures: SharedFlow<BleOperationFailure> = _failures.asSharedFlow()

    var startAdvertisingResult: BleOperationResult = BleOperationResult.Started
    var startAdvertisingFailure: Throwable? = null
    var startScanningResult: BleOperationResult = BleOperationResult.Started

    val startAdvertisingCalls = mutableListOf<BleAdvertisement>()
    var startScanningCallCount = 0
    var stopCallCount = 0
    var stopAdvertisingCallCount = 0
    var stopScanningCallCount = 0

    override fun startAdvertising(advertisement: BleAdvertisement): BleOperationResult {
        startAdvertisingCalls += advertisement
        startAdvertisingFailure?.let { throw it }
        return startAdvertisingResult
    }

    override fun startScanning(): BleOperationResult {
        startScanningCallCount += 1
        return startScanningResult
    }

    override fun stop() {
        stopCallCount += 1
    }

    override fun stopAdvertising() {
        stopAdvertisingCallCount += 1
    }

    override fun stopScanning() {
        stopScanningCallCount += 1
    }

    fun setSessions(value: List<SessionInfo>) {
        _discoveredSessions.value = value
    }

    fun emitFailure(failure: BleOperationFailure) {
        check(_failures.tryEmit(failure)) { "Failed to emit fake BLE failure $failure" }
    }
}
