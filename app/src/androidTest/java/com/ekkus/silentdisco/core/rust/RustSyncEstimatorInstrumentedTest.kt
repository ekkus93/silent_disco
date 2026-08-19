package com.ekkus.silentdisco.core.rust

import android.os.Build
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.double
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class RustSyncEstimatorInstrumentedTest {
    private val json = Json {
        ignoreUnknownKeys = false
    }

    @Test
    fun androidFixtureRunsThroughRustOwnedEstimator() {
        val deviceAbis = Build.SUPPORTED_ABIS.joinToString(separator = ", ")
        val fixture = loadFixture().jsonObject
        val estimator = try {
            RustCoreBridge.openSyncEstimator(
                maxSamples = fixture.getValue("maxSamples").jsonPrimitive.int,
                maxAcceptedRttMs = fixture.getValue("maxAcceptedRttMs").jsonPrimitive.double,
            )
        } catch (error: Throwable) {
            throw AssertionError(
                "Rust synchronization estimator creation failed on device ABIs: $deviceAbis",
                error,
            )
        }

        estimator.use { rustEstimator ->
            fixture.getValue("samples").jsonArray.forEachIndexed { index, element ->
                val sample = element.jsonObject
                val observation = try {
                    rustEstimator.observe(
                        t1LocalSendMs = sample.getValue("t1").jsonPrimitive.long,
                        t2HostReceiveMs = sample.getValue("t2").jsonPrimitive.long,
                        t3HostSendMs = sample.getValue("t3").jsonPrimitive.long,
                        t4LocalReceiveMs = sample.getValue("t4").jsonPrimitive.long,
                    )
                } catch (error: Throwable) {
                    throw AssertionError(
                        "Rust synchronization sample $index failed on device ABIs: $deviceAbis",
                        error,
                    )
                }
                assertClose(
                    label = "sample $index RTT",
                    actual = observation.roundTripTimeMs,
                    expected = sample.getValue("expectedRttMs").jsonPrimitive.double,
                )
                assertClose(
                    label = "sample $index offset",
                    actual = observation.offsetMs,
                    expected = sample.getValue("expectedOffsetMs").jsonPrimitive.double,
                )
                assertEquals(
                    "sample $index acceptance on device ABIs: $deviceAbis",
                    sample.getValue("accepted").jsonPrimitive.boolean,
                    observation.accepted,
                )
            }

            val expected = fixture.getValue("expectedFinalState").jsonObject
            val snapshot = rustEstimator.snapshot()
            assertClose(
                label = "final offset",
                actual = snapshot.offsetMs,
                expected = expected.getValue("offsetMs").jsonPrimitive.double,
            )
            assertClose(
                label = "final RTT",
                actual = snapshot.roundTripTimeMs,
                expected = expected.getValue("rttMs").jsonPrimitive.double,
            )
            assertClose(
                label = "final jitter",
                actual = snapshot.jitterMs,
                expected = expected.getValue("jitterMs").jsonPrimitive.double,
            )
            assertEquals(
                "final confidence on device ABIs: $deviceAbis",
                RustSyncConfidence.valueOf(
                    expected.getValue("confidence").jsonPrimitive.content,
                ),
                snapshot.confidence,
            )
            assertEquals(
                "accepted sample count on device ABIs: $deviceAbis",
                2,
                snapshot.acceptedSampleCount,
            )
            assertTrue(
                "Rust snapshot skew must be finite on device ABIs: $deviceAbis",
                snapshot.skewPpm.isFinite(),
            )
        }
    }


    @Test
    fun boundedAcquisitionMetadataCrossesThePrimitiveBridge() {
        RustCoreBridge.openSyncEstimator().use { estimator ->
            listOf(0L, 250L, 500L).forEach { t1 ->
                val rejected = estimator.observe(
                    t1LocalSendMs = t1,
                    t2HostReceiveMs = t1 + 125,
                    t3HostSendMs = t1 + 125,
                    t4LocalReceiveMs = t1 + 250,
                )
                assertTrue(!rejected.accepted)
            }

            val accepted = estimator.observe(
                t1LocalSendMs = 750,
                t2HostReceiveMs = 875,
                t3HostSendMs = 875,
                t4LocalReceiveMs = 1_000,
            )
            assertTrue(accepted.accepted)
            assertEquals(3L, accepted.acquisitionRejectedSampleCount)
            assertEquals(1_000L, accepted.acquisitionElapsedMs)
            assertClose("acquisition RTT limit", accepted.acquisitionRttLimitMs, 375.0)
            assertTrue(accepted.degradedLock)
        }
    }

    @Test
    fun impossibleTimestampOrderingFailsExplicitly() {
        RustCoreBridge.openSyncEstimator().use { estimator ->
            val error = try {
                estimator.observe(
                    t1LocalSendMs = 100,
                    t2HostReceiveMs = 110,
                    t3HostSendMs = 111,
                    t4LocalReceiveMs = 99,
                )
                throw AssertionError("Impossible timestamp ordering unexpectedly succeeded")
            } catch (error: RustSyncNativeException) {
                error
            }
            assertEquals(-4, error.statusCode)
        }
    }

    private fun loadFixture() = InstrumentationRegistry
        .getInstrumentation()
        .context
        .assets
        .open("rust-migration/sync/clock_sync_v1.json")
        .bufferedReader()
        .use { reader -> json.parseToJsonElement(reader.readText()) }

    private fun assertClose(
        label: String,
        actual: Double,
        expected: Double,
    ) {
        val tolerance = 0.000001
        assertTrue(
            "$label expected $expected but was $actual",
            kotlin.math.abs(actual - expected) <= tolerance,
        )
    }
}
