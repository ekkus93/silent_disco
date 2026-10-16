package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.rust.RustSyncConfidence
import com.google.common.truth.Truth.assertThat
import org.junit.Test

/**
 * Covers the pure Kotlin logic [MainViewModelSynchronization.kt]'s
 * `applySyncResponse` uses to map a [com.ekkus.silentdisco.core.rust.RustSyncEstimator]
 * observation onto [com.ekkus.silentdisco.core.model.SyncState]/UI fields.
 *
 * `RustSyncEstimator` itself cannot be exercised from a JVM unit test: its
 * native library is only packaged into the `debug`/`pocDebug`/`release`
 * Android variants' `jniLibs` source sets (`app/build.gradle.kts`), and the
 * `.so` artifacts it builds are Android-target (bionic libc) binaries that a
 * host JVM cannot load via `System.loadLibrary` regardless. That real
 * estimator is exercised on-device by `RustSyncEstimatorInstrumentedTest`
 * (`app/src/androidTest`) against the identical fixture used by this
 * project's other Rust-migration compatibility fixtures. This test covers
 * everything on the Kotlin side of that boundary instead of standing up a
 * mock that would test nothing real.
 */
class SyncEstimateMappingTest {

    @Test
    fun `sample indexed boundary maps through the accepted host clock offset`() {
        assertThat(
            listenerBoundaryLocalTimeMs(
                hostStartTimeMs = 1_000_000,
                firstSampleIndex = 48_000,
                hostOffsetMs = 125.0,
            ),
        ).isWithin(0.001).of(1_000_875.0)
    }

    @Test
    fun `non packet aligned boundary uses decoded sample time rather than rounded duration`() {
        val boundaryTimeMs = listenerBoundaryLocalTimeMs(
            hostStartTimeMs = 5_000,
            firstSampleIndex = 241,
            hostOffsetMs = 0.0,
        )

        assertThat(boundaryTimeMs).isWithin(0.001).of(5_000 + 241_000.0 / 48_000.0)
    }

    @Test
    fun `host and listener boundary deadlines agree across the sample timeline`() {
        val hostStartMs = 73_000L
        val hostOffsetMs = 186.25
        val boundarySamples = listOf(0L, 1L, 241L, 24_001L, 48_000L, 1_234_567L, 17_280_000L)

        boundarySamples.forEach { firstSampleIndex ->
            // Desktop's scheduler uses integer division; listener presentation
            // retains the fractional millisecond before converting its delay.
            val hostDeadlineMs = hostStartMs + firstSampleIndex * 1_000L / 48_000L
            val listenerDeadlineMs = listenerBoundaryLocalTimeMs(
                hostStartTimeMs = hostStartMs,
                firstSampleIndex = firstSampleIndex,
                hostOffsetMs = hostOffsetMs,
            )
            val hostDeadlineInListenerClock = hostDeadlineMs - hostOffsetMs

            assertThat(listenerDeadlineMs - hostDeadlineInListenerClock)
                .isAtLeast(0.0)
            assertThat(listenerDeadlineMs - hostDeadlineInListenerClock)
                .isLessThan(1.0)
        }
    }

    @Test
    fun `negative host offset advances the local boundary deadline`() {
        assertThat(
            listenerBoundaryLocalTimeMs(
                hostStartTimeMs = 20_000,
                firstSampleIndex = 24_000,
                hostOffsetMs = -40.0,
            ),
        ).isWithin(0.001).of(20_540.0)
    }

    @Test
    fun `RustSyncConfidence maps onto the identical SyncQualityBadge ordinal`() {
        assertThat(RustSyncConfidence.UNKNOWN.toAppSyncQuality()).isEqualTo(SyncQualityBadge.UNKNOWN)
        assertThat(RustSyncConfidence.POOR.toAppSyncQuality()).isEqualTo(SyncQualityBadge.POOR)
        assertThat(RustSyncConfidence.FAIR.toAppSyncQuality()).isEqualTo(SyncQualityBadge.FAIR)
        assertThat(RustSyncConfidence.GOOD.toAppSyncQuality()).isEqualTo(SyncQualityBadge.GOOD)
        assertThat(RustSyncConfidence.EXCELLENT.toAppSyncQuality()).isEqualTo(SyncQualityBadge.EXCELLENT)
    }

    @Test
    fun `initial offset establishes the mapping even when clock epochs differ`() {
        assertThat(
            shouldResyncForOffset(
                offsetMs = 185_355.0,
                previousOffsetMs = null,
                driftThresholdMs = 18.0,
            ),
        ).isFalse()
    }

    @Test
    fun `stable host offset does not trigger resync`() {
        assertThat(
            shouldResyncForOffset(
                offsetMs = 185_355.0,
                previousOffsetMs = 185_354.9,
                driftThresholdMs = 18.0,
            ),
        ).isFalse()
    }

    @Test
    fun `host offset movement beyond threshold triggers resync in either direction`() {
        assertThat(
            shouldResyncForOffset(
                offsetMs = 18.1,
                previousOffsetMs = 0.0,
                driftThresholdMs = 18.0,
            ),
        ).isTrue()
        assertThat(
            shouldResyncForOffset(
                offsetMs = -18.1,
                previousOffsetMs = 0.0,
                driftThresholdMs = 18.0,
            ),
        ).isTrue()
    }

    @Test
    fun `host offset movement exactly at threshold does not trigger resync`() {
        assertThat(
            shouldResyncForOffset(
                offsetMs = 18.0,
                previousOffsetMs = 0.0,
                driftThresholdMs = 18.0,
            ),
        ).isFalse()
    }
}
