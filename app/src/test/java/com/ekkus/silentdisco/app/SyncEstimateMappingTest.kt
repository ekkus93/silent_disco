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
    fun `RustSyncConfidence maps onto the identical SyncQualityBadge ordinal`() {
        assertThat(RustSyncConfidence.UNKNOWN.toAppSyncQuality()).isEqualTo(SyncQualityBadge.UNKNOWN)
        assertThat(RustSyncConfidence.POOR.toAppSyncQuality()).isEqualTo(SyncQualityBadge.POOR)
        assertThat(RustSyncConfidence.FAIR.toAppSyncQuality()).isEqualTo(SyncQualityBadge.FAIR)
        assertThat(RustSyncConfidence.GOOD.toAppSyncQuality()).isEqualTo(SyncQualityBadge.GOOD)
        assertThat(RustSyncConfidence.EXCELLENT.toAppSyncQuality()).isEqualTo(SyncQualityBadge.EXCELLENT)
    }

    @Test
    fun `shouldResyncForOffset is false within the configured drift threshold`() {
        assertThat(shouldResyncForOffset(offsetMs = 0.0, driftThresholdMs = 18.0)).isFalse()
        assertThat(shouldResyncForOffset(offsetMs = 17.9, driftThresholdMs = 18.0)).isFalse()
        assertThat(shouldResyncForOffset(offsetMs = -17.9, driftThresholdMs = 18.0)).isFalse()
    }

    @Test
    fun `shouldResyncForOffset is true once the magnitude exceeds the threshold, either sign`() {
        assertThat(shouldResyncForOffset(offsetMs = 18.1, driftThresholdMs = 18.0)).isTrue()
        assertThat(shouldResyncForOffset(offsetMs = -18.1, driftThresholdMs = 18.0)).isTrue()
    }

    @Test
    fun `shouldResyncForOffset is false exactly at the threshold boundary`() {
        // Matches ListenerSyncControllerTest's original semantics: the
        // comparison is strictly-greater-than, so an offset exactly equal to
        // the threshold does not trigger a resync.
        assertThat(shouldResyncForOffset(offsetMs = 18.0, driftThresholdMs = 18.0)).isFalse()
    }
}
