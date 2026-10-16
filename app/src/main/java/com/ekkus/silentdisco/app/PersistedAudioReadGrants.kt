package com.ekkus.silentdisco.app

import android.content.ContentResolver
import android.content.Intent
import android.content.SharedPreferences
import android.net.Uri

/** Keeps persisted SAF read grants aligned with the live staged-source lifecycle. */
internal class PersistedAudioReadGrants(
    private val resolver: ContentResolver,
    private val preferences: SharedPreferences,
    private val ledgerKey: String = "persisted-read-grants",
) {
    private val lock = Any()

    fun retain(uri: Uri, newlyRetained: MutableSet<Uri>) {
        val alreadyRetained = resolver.persistedUriPermissions.any { it.uri == uri && it.isReadPermission }
        if (!alreadyRetained && uri !in newlyRetained) {
            resolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
            newlyRetained += uri
        }
        synchronized(lock) {
            val grants = preferences.getStringSet(ledgerKey, emptySet()).orEmpty().toMutableSet()
            if (grants.add(uri.toString())) {
                preferences.edit().putStringSet(ledgerKey, grants).apply()
            }
        }
    }

    fun release(uri: Uri) {
        runCatching {
            resolver.releasePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        synchronized(lock) {
            val grants = preferences.getStringSet(ledgerKey, emptySet()).orEmpty().toMutableSet()
            if (grants.remove(uri.toString())) {
                if (grants.isEmpty()) preferences.edit().remove(ledgerKey).apply()
                else preferences.edit().putStringSet(ledgerKey, grants).apply()
            }
        }
    }

    fun releaseOrphans() {
        val grants = synchronized(lock) {
            preferences.getStringSet(ledgerKey, emptySet()).orEmpty().toList()
        }
        grants.forEach { grant -> runCatching { release(Uri.parse(grant)) } }
    }
}
