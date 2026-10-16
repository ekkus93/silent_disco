package com.ekkus.silentdisco

import android.content.Context
import android.net.Uri
import androidx.test.platform.app.InstrumentationRegistry
import com.ekkus.silentdisco.app.PersistedAudioReadGrants
import com.google.common.truth.Truth.assertThat
import java.util.UUID
import org.junit.Test

class PersistedAudioReadGrantsTest {
    @Test
    fun retainsAndReleasesAnActualPersistableProviderGrant() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val target = instrumentation.targetContext
        val testContext = instrumentation.context
        val preferences = target.getSharedPreferences(
            "grant-test-${UUID.randomUUID()}",
            Context.MODE_PRIVATE,
        )
        val uriText = testContext.contentResolver.call(
            Uri.parse("content://${PersistableGrantTestProvider.AUTHORITY}"),
            PersistableGrantTestProvider.GRANT_METHOD,
            "${target.packageName}/${UUID.randomUUID()}",
            null,
        )?.getString(PersistableGrantTestProvider.URI_RESULT_KEY)
        val uri = Uri.parse(requireNotNull(uriText))
        val grants = PersistedAudioReadGrants(target.contentResolver, preferences)

        try {
            grants.retain(uri, mutableSetOf())

            assertThat(target.contentResolver.persistedUriPermissions.any {
                it.uri == uri && it.isReadPermission
            }).isTrue()
            assertThat(preferences.getStringSet("persisted-read-grants", emptySet()).orEmpty())
                .contains(uri.toString())

            grants.release(uri)

            assertThat(target.contentResolver.persistedUriPermissions.any { it.uri == uri }).isFalse()
            assertThat(preferences.getStringSet("persisted-read-grants", emptySet()).orEmpty())
                .doesNotContain(uri.toString())
        } finally {
            grants.release(uri)
            preferences.edit().clear().commit()
        }
    }

    @Test
    fun releasesAnActualPersistableGrantFromTheRecoveryLedger() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val target = instrumentation.targetContext
        val testContext = instrumentation.context
        val preferences = target.getSharedPreferences(
            "grant-recovery-test-${UUID.randomUUID()}",
            Context.MODE_PRIVATE,
        )
        val uri = issuePersistableGrant(testContext, target.packageName)
        val initialOwner = PersistedAudioReadGrants(target.contentResolver, preferences)

        try {
            initialOwner.retain(uri, mutableSetOf())
            val restartedOwner = PersistedAudioReadGrants(target.contentResolver, preferences)
            restartedOwner.releaseOrphans()

            assertThat(target.contentResolver.persistedUriPermissions.any { it.uri == uri }).isFalse()
            assertThat(preferences.getStringSet("persisted-read-grants", emptySet()).orEmpty())
                .doesNotContain(uri.toString())
        } finally {
            initialOwner.release(uri)
            preferences.edit().clear().commit()
        }
    }

    private fun issuePersistableGrant(testContext: Context, targetPackage: String): Uri {
        val uriText = testContext.contentResolver.call(
            Uri.parse("content://${PersistableGrantTestProvider.AUTHORITY}"),
            PersistableGrantTestProvider.GRANT_METHOD,
            "$targetPackage/${UUID.randomUUID()}",
            null,
        )?.getString(PersistableGrantTestProvider.URI_RESULT_KEY)
        return Uri.parse(requireNotNull(uriText))
    }
}
