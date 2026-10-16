package com.ekkus.silentdisco

import android.app.Application
import android.content.Context
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStore
import androidx.lifecycle.ViewModelStoreOwner
import androidx.test.platform.app.InstrumentationRegistry
import com.ekkus.silentdisco.app.MainViewModel
import com.google.common.truth.Truth.assertThat
import java.io.File
import java.util.UUID
import org.junit.Test

class HostSourceRecoveryTest {
    @Test
    fun newViewModelCleansStagedFilesAndGrantLedgerLeftByProcessLoss() {
        val application = InstrumentationRegistry.getInstrumentation()
            .targetContext.applicationContext as Application
        val token = UUID.randomUUID().toString()
        val orphan = File(application.cacheDir, "silent-disco-source-$token.wav")
        orphan.writeBytes(byteArrayOf(1, 2, 3))
        val preferences = application.getSharedPreferences(
            "host-audio-grant-ledger",
            Context.MODE_PRIVATE,
        )
        val orphanUri = "content://test.invalid/audio/$token"
        val previousLedger = preferences.getStringSet("persisted-read-grants", emptySet()).orEmpty()
        preferences.edit()
            .putStringSet("persisted-read-grants", previousLedger + orphanUri)
            .commit()
        val store = ViewModelStore()
        val owner = object : ViewModelStoreOwner {
            override val viewModelStore: ViewModelStore = store
        }

        try {
            val viewModel = ViewModelProvider(
                owner,
                ViewModelProvider.AndroidViewModelFactory.getInstance(application),
            )[MainViewModel::class.java]

            assertThat(viewModel).isNotNull()
            assertThat(orphan.exists()).isFalse()
            assertThat(preferences.getStringSet("persisted-read-grants", emptySet()).orEmpty())
                .doesNotContain(orphanUri)
        } finally {
            store.clear()
        }
    }
}
