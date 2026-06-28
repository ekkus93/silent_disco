package com.ekkus.silentdisco.core.transport

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Test

class BleOperationFailureTest {

    @Test
    fun bleOperationFailure_scan_hasCorrectOperation() {
        val failure = BleOperationFailure(BleOperation.SCAN, "BLE scan failed with code=2")
        assertEquals(BleOperation.SCAN, failure.operation)
        assertEquals("BLE scan failed with code=2", failure.message)
    }

    @Test
    fun bleOperationFailure_advertise_hasCorrectOperation() {
        val failure = BleOperationFailure(BleOperation.ADVERTISE, "BLE advertise failed with code=1")
        assertEquals(BleOperation.ADVERTISE, failure.operation)
        assertEquals("BLE advertise failed with code=1", failure.message)
    }

    @Test
    fun bleFailureFlow_emitsAndCollects() = runBlocking {
        // Use replay=2 so items are buffered before collector subscribes
        val _failures = MutableSharedFlow<BleOperationFailure>(replay = 2, extraBufferCapacity = 8)
        val failures: SharedFlow<BleOperationFailure> = _failures.asSharedFlow()

        val scanFailure = BleOperationFailure(BleOperation.SCAN, "BLE scan failed with code=2")
        val advertiseFailure = BleOperationFailure(BleOperation.ADVERTISE, "BLE advertise failed with code=3")

        _failures.tryEmit(scanFailure)
        _failures.tryEmit(advertiseFailure)

        val collected = mutableListOf<BleOperationFailure>()
        val job = CoroutineScope(Dispatchers.Default).launch {
            failures.collect { collected.add(it) }
        }

        delay(50)
        job.cancel()

        assertEquals(2, collected.size)
        assertEquals(BleOperation.SCAN, collected[0].operation)
        assertEquals(BleOperation.ADVERTISE, collected[1].operation)
    }

    @Test
    fun bleOperationEnum_hasTwoValues() {
        val values = BleOperation.values()
        assertEquals(2, values.size)
        assert(values.contains(BleOperation.SCAN))
        assert(values.contains(BleOperation.ADVERTISE))
    }
}
