package com.ekkus.silentdisco.core.rust

import android.os.Build
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class RustCoreBridgeInstrumentedTest {
    @Test
    fun nativeLibraryLoadsAndReportsSupportedAbiVersion() {
        val deviceAbis = Build.SUPPORTED_ABIS.joinToString(separator = ", ")
        val version = try {
            RustCoreBridge.requireSupportedAbiVersion()
        } catch (error: Throwable) {
            throw AssertionError(
                "Rust core load/version check failed on device ABIs: $deviceAbis",
                error,
            )
        }

        assertEquals(
            "Unexpected Rust core ABI version on device ABIs: $deviceAbis",
            SUPPORTED_RUST_CORE_ABI_VERSION,
            version,
        )
    }
}
