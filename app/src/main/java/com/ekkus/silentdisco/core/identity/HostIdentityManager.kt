package com.ekkus.silentdisco.core.identity

import android.util.Base64
import java.security.KeyPair
import java.security.Signature

/**
 * Owns the device's stable host signing key inside Android Keystore.
 *
 * Private key bytes never enter Kotlin, Rust, QR payloads, logs, or persistence.
 * Rust receives only the X.509 public key and an ES256 signature to validate.
 */
class HostIdentityManager(
    private val alias: String = KEY_ALIAS,
    private val record: HostKeyRecord? = null,
) {
    private val key = KeystoreEcKey(alias)
    @Volatile
    private var pendingChange: HostKeyContinuity.Replaced? = null

    /**
     * Returns and clears a detected key replacement so the UI reports it once.
     * The new key is used for future signatures; the old identity is never
     * trusted on its behalf.
     */
    fun consumeKeyReplacement(): HostKeyContinuity.Replaced? {
        val change = pendingChange
        pendingChange = null
        return change
    }

    /**
     * Startup check that never creates an identity for an install that has
     * not used one: it only runs when a key fingerprint was recorded earlier.
     * Returns a replacement (a missing or different Keystore key) once.
     */
    @Synchronized
    fun checkContinuity(): HostKeyContinuity.Replaced? {
        if (record?.read() == null) return null
        keyPair()
        return consumeKeyReplacement()
    }

    @Synchronized
    fun publicKeyDer(): ByteArray = keyPair().public.encoded.copyOf()

    @Synchronized
    fun signBase64Url(payload: ByteArray): String {
        val signature = Signature.getInstance("SHA256withECDSA").apply {
            initSign(keyPair().private)
            update(payload)
        }
        return Base64.encodeToString(
            signature.sign(),
            Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING,
        )
    }

    private fun keyPair(): KeyPair {
        val pair = loadOrCreateKeyPair()
        record?.let { store ->
            val current = hostKeyFingerprint(pair.public.encoded)
            when (val outcome = assessHostKeyContinuity(store.read(), current)) {
                HostKeyContinuity.Unchanged -> Unit
                HostKeyContinuity.FirstUse -> store.write(current)
                is HostKeyContinuity.Replaced -> {
                    pendingChange = outcome
                    store.write(current)
                }
            }
        }
        return pair
    }

    private fun loadOrCreateKeyPair(): KeyPair = key.keyPair()

    companion object {
        private const val KEY_ALIAS = "silent-disco-host-identity-v1"
    }
}
