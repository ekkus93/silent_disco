package com.ekkus.silentdisco.core.identity

import com.ekkus.silentdisco.core.uniffi.FfiBridgeException
import com.ekkus.silentdisco.core.uniffi.FfiListenerProofSigner

/**
 * Lets the Rust host sign its TLS handshakes with the Keystore TLS key. Failures
 * reach Rust as typed callback errors, so the host refuses to bind rather than
 * running without a TLS identity.
 */
class KeystoreHostTlsSigner(
    private val identity: HostTlsIdentityManager = HostTlsIdentityManager(),
) : FfiListenerProofSigner {
    override fun spkiDer(): ByteArray = guard("read the host TLS key") { identity.publicKeyDer() }

    override fun signDer(transcript: ByteArray): ByteArray =
        guard("sign the host TLS handshake") { identity.signDer(transcript) }

    private inline fun <T> guard(operation: String, block: () -> T): T =
        try {
            block()
        } catch (error: FfiBridgeException) {
            throw error
        } catch (error: Exception) {
            throw FfiBridgeException.Callback("$operation failed: ${error.message}")
        }
}
