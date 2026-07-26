package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.rust.RustDatabaseException
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class StorageInitializationPolicyTest {
    @Test
    fun migrationFailureProducesFatalVisibleState() {
        val error = RustDatabaseException(-103, "open database")

        val state = AppUiState().withStorageInitializationFailure(error)

        assertThat(state.storageState).isEqualTo(StorageInitializationState.FATAL_FAILURE)
        assertThat(state.storageError).contains("Fatal persistent storage failure")
        assertThat(state.lastError).isEqualTo(state.storageError)
        assertThat(state.persistentFeaturesEnabled()).isFalse()
        assertThat(state.canRetryStorageInitialization()).isFalse()
    }

    @Test
    fun busyFailureProducesRecoverableRetryableState() {
        val error = RustDatabaseException(-105, "open database")

        val state = AppUiState().withStorageInitializationFailure(error)

        assertThat(state.storageState).isEqualTo(StorageInitializationState.RECOVERABLE_FAILURE)
        assertThat(state.storageError).contains("temporarily unavailable")
        assertThat(state.canRetryStorageInitialization()).isTrue()
        assertThat(state.persistentFeaturesEnabled()).isFalse()
    }

    @Test
    fun readyStateEnablesPersistenceDependentFeatures() {
        val state = AppUiState(storageState = StorageInitializationState.READY)

        assertThat(state.persistentFeaturesEnabled()).isTrue()
        assertThat(state.canRetryStorageInitialization()).isFalse()
        assertThat(state.storageStatusLabel()).isEqualTo("Rust storage ready")
    }
}
