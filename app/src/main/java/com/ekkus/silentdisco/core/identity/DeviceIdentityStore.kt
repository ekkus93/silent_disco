package com.ekkus.silentdisco.core.identity

import android.content.Context
import androidx.core.content.edit
import java.util.UUID

/**
 * A stable, per-install device identifier: generated once on first use and
 * persisted thereafter in app-private [android.content.SharedPreferences].
 *
 * Used as the `device_id` for both host and listener roles' shared-core
 * actor identity and join requests. Previously both roles used a hardcoded
 * literal string (`"listener-device"`, `"android-host-device"`), so every
 * install of the app presented the identical identity to any host,
 * indistinguishable from any other install. That silently merged two
 * distinct listeners into one from the host's point of view: confirmed
 * empirically against two real Android devices joining the same session --
 * both join requests were received and individually approved, but the
 * host's snapshot only ever reported one connected listener, because
 * listener admission keys on `device_id`.
 *
 * Not a cryptographic identity -- see [HostIdentityManager] for the
 * Keystore-backed signing key used for QR/invite verification, a separate
 * concern with a separate lifecycle. This is a plain, unauthenticated
 * label the host uses to tell listeners apart.
 */
object DeviceIdentityStore {
    private const val PREFS_NAME = "device_identity"
    private const val KEY_DEVICE_ID = "device_id"

    @Volatile
    private var cached: String? = null

    /**
     * Returns this install's stable device id, generating and persisting a
     * new one on first call. Safe to call from any role (host or
     * listener); both share the same underlying identity, matching a
     * physical device having exactly one identity regardless of which
     * role it is currently playing.
     */
    fun get(context: Context): String {
        cached?.let { return it }
        synchronized(this) {
            cached?.let { return it }
            val prefs = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
            val id = prefs.getString(KEY_DEVICE_ID, null) ?: UUID.randomUUID().toString().also { generated ->
                prefs.edit { putString(KEY_DEVICE_ID, generated) }
            }
            cached = id
            return id
        }
    }
}
