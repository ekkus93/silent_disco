package com.ekkus.silentdisco.core.identity

import com.ekkus.silentdisco.core.uniffi.FfiProofHostListener
import com.ekkus.silentdisco.core.uniffi.FfiVerifiedProofListener
import com.ekkus.silentdisco.core.uniffi.listenerProveToHost
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.MessageDigest
import java.security.spec.ECGenParameterSpec
import java.util.UUID
import java.util.concurrent.atomic.AtomicReference
import kotlin.concurrent.thread
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Test

/**
 * The listener proof end to end on a real device: the Keystore listener key
 * signs through the FFI callback, the Rust listener proves over TLS 1.3 pinned to
 * the host key, and the Rust host verifies. The host side is the same core
 * verifier a desktop host would run.
 */
class ListenerProofOverFfiInstrumentedTest {
    private val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
    private val alias = "silent-disco-listener-ffi-test-${UUID.randomUUID()}"

    @After
    fun deleteKey() {
        runCatching { keyStore.deleteEntry(alias) }
    }

    private fun hostPkcs8(): ByteArray =
        KeyPairGenerator.getInstance("EC")
            .apply { initialize(ECGenParameterSpec("secp256r1")) }
            .generateKeyPair()
            .private
            .encoded

    private fun sha256(bytes: ByteArray): ByteArray = MessageDigest.getInstance("SHA-256").digest(bytes)

    /** Starts a host that accepts one listener on a background thread. */
    private class PendingHost(val host: FfiProofHostListener, session: String) {
        val verified = AtomicReference<FfiVerifiedProofListener?>()
        val failure = AtomicReference<Throwable?>()
        private val worker = thread {
            try {
                verified.set(host.acceptAndVerify(session, 1UL))
            } catch (error: Throwable) {
                failure.set(error)
            }
        }

        fun finish(): Pair<FfiVerifiedProofListener?, Throwable?> {
            worker.join(15_000)
            return verified.get() to failure.get()
        }
    }

    @Test
    fun theKeystoreListenerKeyIsVerifiedByTheHostOverTls() {
        val session = "ffi-proof-${UUID.randomUUID()}"
        val host = FfiProofHostListener.bind("127.0.0.1:0", hostPkcs8())
        val manager = ListenerIdentityManager(alias)
        val pending = PendingHost(host, session)

        listenerProveToHost(
            "127.0.0.1:${host.localPort()}",
            host.hostSpki(),
            session,
            1UL,
            KeystoreListenerProofSigner(manager),
        )
        val (verified, failure) = pending.finish()

        assertNull("the host must accept the proof: $failure", failure)
        assertNotNull(verified)
        assertArrayEquals(sha256(manager.publicKeyDer()), verified!!.listenerFingerprint)
        assertArrayEquals(sha256(host.hostSpki()), verified.hostFingerprint)
    }

    @Test
    fun aListenerPinnedToAnotherHostKeyNeverProves() {
        val session = "ffi-pin-${UUID.randomUUID()}"
        val host = FfiProofHostListener.bind("127.0.0.1:0", hostPkcs8())
        val manager = ListenerIdentityManager(alias)
        val pending = PendingHost(host, session)

        // Pin the listener's own key as if it were the host's: the handshake must fail.
        assertThrows(com.ekkus.silentdisco.core.uniffi.FfiBridgeException::class.java) {
            listenerProveToHost(
                "127.0.0.1:${host.localPort()}",
                manager.publicKeyDer(),
                session,
                1UL,
                KeystoreListenerProofSigner(manager),
            )
        }
        val (verified, failure) = pending.finish()
        assertNull(verified)
        assertNotNull("the host must reject the failed handshake", failure)
    }

    @Test
    fun aSignatureOverTheWrongTranscriptIsRefusedByTheHost() {
        val session = "ffi-mutant-${UUID.randomUUID()}"
        val host = FfiProofHostListener.bind("127.0.0.1:0", hostPkcs8())
        val manager = ListenerIdentityManager(alias)
        val pending = PendingHost(host, session)
        // Deliberately sign zeros of the same length instead of the transcript.
        val wrongInput = object : com.ekkus.silentdisco.core.uniffi.FfiListenerProofSigner {
            override fun spkiDer(): ByteArray = manager.publicKeyDer()
            override fun signDer(transcript: ByteArray): ByteArray = manager.signDer(ByteArray(transcript.size))
        }

        runCatching {
            listenerProveToHost("127.0.0.1:${host.localPort()}", host.hostSpki(), session, 1UL, wrongInput)
        }
        val (verified, failure) = pending.finish()
        assertNull("a signature over the wrong transcript was accepted", verified)
        assertNotNull(failure)
    }
}
