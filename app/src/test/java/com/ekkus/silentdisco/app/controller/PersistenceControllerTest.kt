package com.ekkus.silentdisco.app.controller

import android.content.ContentResolver
import android.content.SharedPreferences
import com.ekkus.silentdisco.app.PersistedAudioReadGrants
import com.ekkus.silentdisco.core.rust.FakeRustDomainStore
import com.ekkus.silentdisco.core.rust.RustDatabaseException
import com.ekkus.silentdisco.core.rust.RustStoredTuningSettings
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Test
import org.mockito.kotlin.any
import org.mockito.kotlin.anyOrNull
import org.mockito.kotlin.mock
import org.mockito.kotlin.whenever

@OptIn(ExperimentalCoroutinesApi::class)
class PersistenceControllerTest {
    private fun grants(): PersistedAudioReadGrants {
        val preferences = mock<SharedPreferences>()
        whenever(preferences.getStringSet(any(), anyOrNull())).thenReturn(emptySet())
        return PersistedAudioReadGrants(mock<ContentResolver>(), preferences)
    }

    @Test
    fun initializeReportsTheStoredTuningOrTheFailure() = runTest {
        val parent = CoroutineScope(SupervisorJob() + StandardTestDispatcher(testScheduler))
        val store = FakeRustDomainStore()
        val persistence = PersistenceController(parent, store, grants())
        val results = mutableListOf<Result<RustStoredTuningSettings>>()

        persistence.initialize { results += it }
        advanceUntilIdle()
        store.initializeFailure = RustDatabaseException(-105, "open database")
        persistence.initialize { results += it }
        advanceUntilIdle()

        assertThat(results).hasSize(2)
        assertThat(results[0].isSuccess).isTrue()
        assertThat(results[1].exceptionOrNull()).isInstanceOf(RustDatabaseException::class.java)
        assertThat(store.initializeCallCount).isEqualTo(2)
        parent.cancel()
    }

    @Test
    fun theStoreClosesOnceWhetherTeardownOrTheRegistryGetsThereFirst() = runTest {
        val parent = CoroutineScope(SupervisorJob() + StandardTestDispatcher(testScheduler))
        val explicit = FakeRustDomainStore()
        PersistenceController(parent, explicit, grants()).apply {
            closeStore()
            close()
        }
        assertThat(explicit.closeCallCount).isEqualTo(1)

        val registryOnly = FakeRustDomainStore()
        PersistenceController(parent, registryOnly, grants()).close()
        assertThat(registryOnly.closeCallCount).isEqualTo(1)
        parent.cancel()
    }
}
