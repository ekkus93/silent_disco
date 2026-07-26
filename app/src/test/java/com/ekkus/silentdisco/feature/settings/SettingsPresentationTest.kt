package com.ekkus.silentdisco.feature.settings

import com.ekkus.silentdisco.app.StorageInitializationState
import com.ekkus.silentdisco.ui.components.StatusTone
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class SettingsPresentationTest {
    @Test
    fun readyStorageIsPositive() {
        assertThat(settingsStorageLabel(StorageInitializationState.READY)).isEqualTo("Available")
        assertThat(settingsStorageTone(StorageInitializationState.READY)).isEqualTo(StatusTone.POSITIVE)
    }

    @Test
    fun recoverableAndFatalFailuresStayDistinct() {
        assertThat(settingsStorageLabel(StorageInitializationState.RECOVERABLE_FAILURE))
            .isEqualTo("Temporarily unavailable")
        assertThat(settingsStorageTone(StorageInitializationState.RECOVERABLE_FAILURE))
            .isEqualTo(StatusTone.ATTENTION)
        assertThat(settingsStorageLabel(StorageInitializationState.FATAL_FAILURE))
            .isEqualTo("Could not be opened")
        assertThat(settingsStorageTone(StorageInitializationState.FATAL_FAILURE))
            .isEqualTo(StatusTone.CRITICAL)
    }
}
