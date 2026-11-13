package com.ekkus.silentdisco.app.controller

import android.net.Uri
import com.ekkus.silentdisco.app.PersistedAudioReadGrants
import com.ekkus.silentdisco.core.rust.RustDomainStore
import com.ekkus.silentdisco.core.rust.RustStoredTuningSettings
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Persistence: the Rust-owned domain store and the audio read grants the app
 * keeps across restarts (God-file refactor spec §4). Nothing else holds
 * either. Rust still owns all domain SQL; this only opens, saves through and
 * closes the store.
 *
 * Storage status shown to the user stays the coordinator's: [initialize]
 * reports its result, and the coordinator maps it into the UI state as
 * before.
 */
internal class PersistenceController(
    parentScope: CoroutineScope,
    private val store: RustDomainStore,
    private val readGrants: PersistedAudioReadGrants,
) : ViewModelController(parentScope) {
    private val closed = AtomicBoolean(false)

    /** Opens the store and reports the stored tuning, or the failure. */
    fun initialize(onResult: (Result<RustStoredTuningSettings>) -> Unit) {
        scope.launch { onResult(runCatching { store.initialize() }) }
    }

    suspend fun saveTuning(settings: RustStoredTuningSettings) = store.saveTuning(settings)

    fun retainAudioReadGrant(uri: Uri, newlyRetained: MutableSet<Uri>) =
        readGrants.retain(uri, newlyRetained)

    fun releaseAudioReadGrant(uri: Uri) = readGrants.release(uri)

    fun releaseOrphanedAudioReadGrants() = readGrants.releaseOrphans()

    /**
     * Closes the store once. `MainViewModel.onCleared` calls this at its
     * original point; [release] only covers a close that never happened.
     */
    suspend fun closeStore() {
        if (closed.compareAndSet(false, true)) store.close()
    }

    override fun release() {
        if (!closed.get()) runBlocking(Dispatchers.IO) { closeStore() }
    }
}
