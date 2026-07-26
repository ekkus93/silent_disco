package com.ekkus.silentdisco.platform.persistence

import android.content.Context
import android.database.sqlite.SQLiteDatabase
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.StorageInitializationState
import com.ekkus.silentdisco.app.withStorageInitializationFailure
import com.ekkus.silentdisco.core.persistence.LegacyPreferencesContract
import com.ekkus.silentdisco.core.rust.RustDatabaseException
import java.io.File
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class AndroidRustDomainStoreInstrumentedTest {
    private val context: Context = ApplicationProvider.getApplicationContext()

    @Test
    fun firstRunImportsSettingsAndTrustThenReopenLoadsRustValues() = runBlocking {
        val suffix = System.nanoTime().toString()
        val preferencesName = "block9-import-$suffix"
        val databaseFileName = "block9-import-$suffix.sqlite3"
        val preferences = context.getSharedPreferences(preferencesName, Context.MODE_PRIVATE)
        val trustedId = "listener-é-$suffix"
        preferences.edit()
            .putInt(LegacyPreferencesContract.SYNC_SAMPLE_WINDOW, 20)
            .putLong(LegacyPreferencesContract.SYNC_CADENCE_MS, 1_750L)
            .putLong(LegacyPreferencesContract.STARTUP_BUFFER_MS, 550L)
            .putLong(LegacyPreferencesContract.LATE_PACKET_THRESHOLD_MS, 55L)
            .putLong(LegacyPreferencesContract.HARD_RESYNC_THRESHOLD_MS, 180L)
            .putLong(LegacyPreferencesContract.SYNC_DRIFT_THRESHOLD_BITS, 24.5.toBits())
            .putBoolean(LegacyPreferencesContract.trustedDeviceKey(trustedId), true)
            .commit()
        val provider = AndroidDatabasePathProvider(context, databaseFileName)
        val store = AndroidRustDomainStore(
            context = context,
            pathProvider = provider,
            preferencesName = preferencesName,
        )
        try {
            val imported = store.initialize()
            assertEquals(20, imported.syncSampleWindow)
            assertEquals(1_750L, imported.syncCadenceMs)
            assertEquals(24.5, imported.syncDriftThresholdMs, 0.0)
            assertTrue(store.isTrusted(trustedId))
            assertTrue(File(provider.databasePath()).isFile)
            assertTrue(
                LegacyPreferencesContract.tuningKeys.none(preferences::contains),
            )
            assertFalse(
                preferences.contains(LegacyPreferencesContract.trustedDeviceKey(trustedId)),
            )
        } finally {
            store.close()
        }

        val reopened = AndroidRustDomainStore(
            context = context,
            pathProvider = provider,
            preferencesName = preferencesName,
        )
        try {
            val settings = reopened.initialize()
            assertEquals(20, settings.syncSampleWindow)
            assertEquals(550L, settings.startupBufferMs)
            assertTrue(reopened.isTrusted(trustedId))
        } finally {
            reopened.close()
            cleanup(preferencesName, provider.databasePath())
        }
    }

    @Test
    fun invalidLegacyTrustPreservesPreferencesAndSurfacesFailure() {
        val suffix = System.nanoTime().toString()
        val preferencesName = "block9-invalid-$suffix"
        val provider = AndroidDatabasePathProvider(context, "block9-invalid-$suffix.sqlite3")
        val preferences = context.getSharedPreferences(preferencesName, Context.MODE_PRIVATE)
        val invalidKey = LegacyPreferencesContract.trustedDeviceKey(" invalid-device ")
        preferences.edit()
            .putInt(LegacyPreferencesContract.SYNC_SAMPLE_WINDOW, 18)
            .putBoolean(invalidKey, true)
            .commit()
        val store = AndroidRustDomainStore(
            context = context,
            pathProvider = provider,
            preferencesName = preferencesName,
        )
        try {
            assertThrows(IllegalStateException::class.java) {
                runBlocking { store.initialize() }
            }
            assertEquals(18, preferences.getInt(LegacyPreferencesContract.SYNC_SAMPLE_WINDOW, -1))
            assertTrue(preferences.getBoolean(invalidKey, false))
        } finally {
            runBlocking { store.close() }
            cleanup(preferencesName, provider.databasePath())
        }
    }

    @Test
    fun malformedLegacyTrustValueIsVisibleAndPreserved() {
        val suffix = System.nanoTime().toString()
        val preferencesName = "block9-malformed-trust-$suffix"
        val provider = AndroidDatabasePathProvider(
            context,
            "block9-malformed-trust-$suffix.sqlite3",
        )
        val preferences = context.getSharedPreferences(preferencesName, Context.MODE_PRIVATE)
        val malformedKey = LegacyPreferencesContract.trustedDeviceKey("listener-$suffix")
        preferences.edit()
            .putString(malformedKey, "not-a-boolean")
            .commit()
        val store = AndroidRustDomainStore(
            context = context,
            pathProvider = provider,
            preferencesName = preferencesName,
        )
        try {
            val error = assertThrows(AndroidRustDomainStoreException::class.java) {
                runBlocking { store.initialize() }
            }
            assertTrue(error.message.orEmpty().contains("does not contain a Boolean"))
            assertEquals("not-a-boolean", preferences.getString(malformedKey, null))
        } finally {
            runBlocking { store.close() }
            cleanup(preferencesName, provider.databasePath())
        }
    }

    @Test
    fun corruptDatabaseFailureIsVisibleAndLeavesLegacyValuesIntact() {
        val suffix = System.nanoTime().toString()
        val preferencesName = "block9-corrupt-$suffix"
        val provider = AndroidDatabasePathProvider(context, "block9-corrupt-$suffix.sqlite3")
        val path = File(provider.databasePath())
        path.writeBytes("not-a-sqlite-database".encodeToByteArray())
        val preferences = context.getSharedPreferences(preferencesName, Context.MODE_PRIVATE)
        preferences.edit()
            .putLong(LegacyPreferencesContract.SYNC_CADENCE_MS, 1_500L)
            .commit()
        val store = AndroidRustDomainStore(
            context = context,
            pathProvider = provider,
            preferencesName = preferencesName,
        )
        try {
            assertThrows(IllegalStateException::class.java) {
                runBlocking { store.initialize() }
            }
            assertEquals(
                1_500L,
                preferences.getLong(LegacyPreferencesContract.SYNC_CADENCE_MS, -1L),
            )
        } finally {
            runBlocking { store.close() }
            cleanup(preferencesName, provider.databasePath())
        }
    }

    @Test
    fun migrationChecksumFailureProducesFatalVisibleStorageState() {
        val suffix = System.nanoTime().toString()
        val preferencesName = "block9-migration-$suffix"
        val provider = AndroidDatabasePathProvider(
            context,
            "block9-migration-$suffix.sqlite3",
        )
        val firstOpen = AndroidRustDomainStore(
            context = context,
            pathProvider = provider,
            preferencesName = preferencesName,
        )
        runBlocking {
            firstOpen.initialize()
            firstOpen.close()
        }

        val databasePath = provider.databasePath()
        SQLiteDatabase.openDatabase(
            databasePath,
            null,
            SQLiteDatabase.OPEN_READWRITE,
        ).use { database ->
            database.execSQL(
                "UPDATE schema_migrations SET checksum = 'tampered' WHERE version = 1",
            )
        }

        val reopened = AndroidRustDomainStore(
            context = context,
            pathProvider = provider,
            preferencesName = preferencesName,
        )
        try {
            val error = assertThrows(RustDatabaseException::class.java) {
                runBlocking { reopened.initialize() }
            }
            assertEquals(-103, error.statusCode)
            val visibleState = AppUiState().withStorageInitializationFailure(error)
            assertEquals(StorageInitializationState.FATAL_FAILURE, visibleState.storageState)
            assertTrue(
                visibleState.storageError.orEmpty().contains("Fatal persistent storage failure"),
            )
            assertEquals(visibleState.storageError, visibleState.lastError)
        } finally {
            runBlocking { reopened.close() }
            cleanup(preferencesName, databasePath)
        }
    }

    private fun cleanup(
        preferencesName: String,
        databasePath: String,
    ) {
        context.getSharedPreferences(preferencesName, Context.MODE_PRIVATE).edit().clear().commit()
        listOf(
            databasePath,
            "$databasePath-wal",
            "$databasePath-shm",
        ).forEach { File(it).delete() }
    }
}
