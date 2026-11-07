package com.ekkus.silentdisco.core.identity

import com.ekkus.silentdisco.core.uniffi.FfiBridgeException
import com.ekkus.silentdisco.core.uniffi.FfiListenerProofSigner

/**
 * Lets the Rust listener proof sign with the Keystore listener key. The key
 * never leaves Keystore: Rust asks for the public key and for signatures over
 * each transcript, and any failure reaches Rust as a typed callback error
 * rather than an empty proof.
 */
class KeystoreListenerProofSigner(
    private val identity: ListenerIdentityManager,
) : FfiListenerProofSigner {
    override fun spkiDer(): ByteArray = guard("read the listener key") { identity.publicKeyDer() }

    override fun signDer(transcript: ByteArray): ByteArray =
        guard("sign the listener proof") { identity.signDer(transcript) }

    private inline fun <T> guard(operation: String, block: () -> T): T =
        try {
            block()
        } catch (error: FfiBridgeException) {
            throw error
        } catch (error: Exception) {
            throw FfiBridgeException.Callback("$operation failed: ${error.message}")
        }
}
