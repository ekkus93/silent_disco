package com.ekkus.silentdisco.core.transport

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.take
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class BleDiscoveryServiceInstrumentedTest {

    private lateinit var service: BleDiscoveryService

    @Before
    fun setUp() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        service = BleDiscoveryService(context)
    }

    @Test
    fun emitScanFailureForTest_emitsToFailuresFlow() = runBlocking {
        val events = mutableListOf<BleOperationFailure>()
        val job = launch(Dispatchers.Default) {
            service.failures.take(1).toList(events)
        }
        delay(50) // ensure collector is subscribed before emission
        service.emitScanFailureForTest(2)
        withTimeout(5_000) { job.join() }

        assertEquals(1, events.size)
        assertEquals(BleOperation.SCAN, events.single().operation)
        assertTrue(
            "Scan failure message should contain error code",
            events.single().message.contains("code=2"),
        )
    }

    @Test
    fun emitAdvertiseFailureForTest_emitsToFailuresFlow() = runBlocking {
        val events = mutableListOf<BleOperationFailure>()
        val job = launch(Dispatchers.Default) {
            service.failures.take(1).toList(events)
        }
        delay(50) // ensure collector is subscribed before emission
        service.emitAdvertiseFailureForTest(3)
        withTimeout(5_000) { job.join() }

        assertEquals(1, events.size)
        assertEquals(BleOperation.ADVERTISE, events.single().operation)
        assertTrue(
            "Advertise failure message should contain error code",
            events.single().message.contains("code=3"),
        )
    }

    @Test
    fun emitScanFailureForTest_clearsDiscoveredSessions() = runBlocking {
        val job = launch(Dispatchers.Default) {
            service.failures.take(1).toList(mutableListOf())
        }
        delay(50)
        service.emitScanFailureForTest(1)
        withTimeout(5_000) { job.join() }

        assertTrue(
            "Discovered sessions should be empty after scan failure",
            service.discoveredSessions.value.isEmpty(),
        )
    }
}
