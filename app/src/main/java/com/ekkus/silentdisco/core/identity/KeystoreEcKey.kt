package com.ekkus.silentdisco.core.identity

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.security.KeyPair
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.Signature
import java.security.spec.ECGenParameterSpec

/**
 * A non-exportable P-256 signing key in Android Keystore, identified by [alias].
 *
 * The private key never leaves Keystore: [privateKey.encoded] is null and only
 * signatures and the X.509 public key cross into app code. A missing key is
 * generated once; an existing key is never replaced here. Callers that need
 * to detect replacement compare fingerprints themselves.
 */
internal class KeystoreEcKey(private val alias: String) {
    @Synchronized
    fun keyPair(): KeyPair {
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

    /** SubjectPublicKeyInfo DER of the key. */
    fun publicKeyDer(): ByteArray = keyPair().public.encoded.copyOf()

    /** ECDSA-SHA256 signature over [payload], in DER form as the core verifier expects. */
    @Synchronized
    fun signDer(payload: ByteArray): ByteArray {
        val signature = Signature.getInstance("SHA256withECDSA").apply {
            initSign(keyPair().private)
            update(payload)
        }
        return signature.sign()
    }

    private companion object {
        const val ANDROID_KEY_STORE = "AndroidKeyStore"
    }
}
