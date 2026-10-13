package com.ekkus.silentdisco.core.transport

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.google.common.truth.Truth.assertThat
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Regression guard for Android <= 30 discovery. Those releases still enforce
 * the legacy BLUETOOTH/BLUETOOTH_ADMIN manifest permissions even though newer
 * releases use the granular BLUETOOTH_* runtime permissions. Run on the
 * project's managed API-29 device so the installed package is inspected on a
 * platform where the declarations are operationally required.
 */
@RunWith(AndroidJUnit4::class)
class LegacyBluetoothManifestInstrumentedTest {
    @Test
    fun installedPackageDeclaresLegacyBluetoothPermissions() {
        assumeTrue("Legacy permissions are only operational through API 30", Build.VERSION.SDK_INT <= 30)
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val packageInfo = context.packageManager.getPackageInfo(
            context.packageName,
            PackageManager.GET_PERMISSIONS,
        )
        val requested = packageInfo.requestedPermissions.orEmpty().toSet()

        assertThat(requested).contains(Manifest.permission.BLUETOOTH)
        assertThat(requested).contains(Manifest.permission.BLUETOOTH_ADMIN)
    }
}
