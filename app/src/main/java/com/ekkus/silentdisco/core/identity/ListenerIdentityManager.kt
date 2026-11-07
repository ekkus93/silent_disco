package com.ekkus.silentdisco.core.identity

/**
 * The listener's proof key: the key a listener signs the host's possession
 * challenge with (see the core `secure_listener_proof` module).
 *
 * It is separate from [HostIdentityManager] so that a device acting as a
 * listener does not present the host's identity, and the reverse.
 */
class ListenerIdentityManager(
    alias: String = KEY_ALIAS,
) {
    private val key = KeystoreEcKey(alias)

    /** SubjectPublicKeyInfo DER of the listener proof key. */
    fun publicKeyDer(): ByteArray = key.publicKeyDer()

    /** DER ECDSA-SHA256 signature over a proof transcript. */
    fun signDer(transcript: ByteArray): ByteArray = key.signDer(transcript)

    companion object {
        const val KEY_ALIAS = "silent-disco-listener-proof-v1"
    }
}
