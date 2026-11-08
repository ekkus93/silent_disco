package com.ekkus.silentdisco.core.identity

/**
 * Owns the host's TLS control-channel key inside Android Keystore. It is a
 * separate key from the invitation-signing key: its public half is signed into
 * each invitation as `tk`, so a listener pins what its handshake proves.
 *
 * The private key never leaves Keystore. Rust receives only the X.509 public
 * key and DER ECDSA-SHA256 signatures over handshake transcripts.
 */
class HostTlsIdentityManager(private val alias: String = KEY_ALIAS) {
    private val key = KeystoreEcKey(alias)

    /** SubjectPublicKeyInfo DER of the TLS key, as the invitation carries it. */
    fun publicKeyDer(): ByteArray = key.publicKeyDer()

    /** DER ECDSA-SHA256 signature over [transcript]. */
    fun signDer(transcript: ByteArray): ByteArray = key.signDer(transcript)

    companion object {
        const val KEY_ALIAS = "silent-disco-host-tls-v1"
    }
}
