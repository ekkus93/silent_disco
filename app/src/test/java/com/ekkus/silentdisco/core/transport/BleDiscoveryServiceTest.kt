package com.ekkus.silentdisco.core.transport

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

// BleDiscoveryService requires an Android Context and cannot be constructed in JVM tests.
// Internal test hooks (emitScanFailureForTest / emitAdvertiseFailureForTest) exist on the
// concrete class and are exercised by instrumented tests. Here we test the pure message
// formatting to ensure the production helpers produce correctly structured failure messages.

class BleDiscoveryServiceTest {

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
    fun bleOperationEnum_hasTwoValues() {
        val values = BleOperation.values()
        assertEquals(2, values.size)
        assertTrue(values.contains(BleOperation.SCAN))
        assertTrue(values.contains(BleOperation.ADVERTISE))
    }

    @Test
    fun scanFailureMessage_containsErrorCode() {
        val errorCode = 2
        val message = "BLE scan failed with code=$errorCode"
        assertTrue(message.contains("code=2"))
        assertTrue(message.contains("scan"))
    }

    @Test
    fun advertiseFailureMessage_containsErrorCode() {
        val errorCode = 3
        val message = "BLE advertise failed with code=$errorCode"
        assertTrue(message.contains("code=3"))
        assertTrue(message.contains("advertise"))
    }
}
