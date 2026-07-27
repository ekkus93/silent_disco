package com.ekkus.silentdisco.core.identity

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyPair
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.Signature
import java.security.spec.ECGenParameterSpec

/**
 * Owns the device's stable host signing key inside Android Keystore.
 *
 * Private key bytes never enter Kotlin, Rust, QR payloads, logs, or persistence.
 * Rust receives only the X.509 public key and an ES256 signature to validate.
 */
class HostIdentityManager(
    private val alias: String = KEY_ALIAS,
) {
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
        val keyStore = KeyStore.getInstance(ANDROID_KEY_STORE).apply { load(null) }
        val existing = keyStore.getEntry(alias, null) as? KeyStore.PrivateKeyEntry
        if (existing != null) return KeyPair(existing.certificate.publicKey, existing.privateKey)

        val generator = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, ANDROID_KEY_STORE)
        generator.initialize(
            KeyGenParameterSpec.Builder(
                alias,
                KeyProperties.PURPOSE_SIGN or KeyProperties.PURPOSE_VERIFY,
            )
                .setAlgorithmParameterSpec(ECGenParameterSpec("secp256r1"))
                .setDigests(KeyProperties.DIGEST_SHA256)
                .setUserAuthenticationRequired(false)
                .build(),
        )
        return generator.generateKeyPair()
    }

    companion object {
        private const val ANDROID_KEY_STORE = "AndroidKeyStore"
        private const val KEY_ALIAS = "silent-disco-host-identity-v1"
    }
}
