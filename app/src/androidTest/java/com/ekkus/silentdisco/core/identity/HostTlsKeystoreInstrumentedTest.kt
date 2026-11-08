package com.ekkus.silentdisco.core.identity

import com.ekkus.silentdisco.core.uniffi.FfiProofHostListener
import com.ekkus.silentdisco.core.uniffi.FfiVerifiedProofListener
import com.ekkus.silentdisco.core.uniffi.listenerProveToHost
import java.security.KeyStore
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
 * The host's TLS control-channel key on a real device. The key stays in Android
 * Keystore; the Rust host presents its SPKI and signs every handshake through the
 * platform callback. A listener pinned to that SPKI must complete TLS 1.3 and its
 * proof, and a listener pinned to any other key must be refused.
 */
class HostTlsKeystoreInstrumentedTest {
    private val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
    private val hostAlias = "silent-disco-host-tls-test-${UUID.randomUUID()}"
    private val listenerAlias = "silent-disco-listener-tls-test-${UUID.randomUUID()}"

    @After
    fun deleteKeys() {
        runCatching { keyStore.deleteEntry(hostAlias) }
        runCatching { keyStore.deleteEntry(listenerAlias) }
    }

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
    fun aPinnedListenerProvesToTheHostWhoseTlsKeyStaysInKeystore() {
        val session = "host-tls-${UUID.randomUUID()}"
        val hostSigner = KeystoreHostTlsSigner(HostTlsIdentityManager(hostAlias))
        val host = FfiProofHostListener.bindWithPlatformKey("127.0.0.1:0", hostSigner)
        val pending = PendingHost(host, session)

        listenerProveToHost(
            "127.0.0.1:${host.localPort()}",
            host.hostSpki(),
            session,
            1UL,
            KeystoreListenerProofSigner(ListenerIdentityManager(listenerAlias)),
        )
        val (verified, failure) = pending.finish()

        assertNull("the host must verify the listener: $failure", failure)
        assertNotNull(verified)
        // The SPKI the host presents is the Keystore TLS key the invitation carries.
        assertArrayEquals(HostTlsIdentityManager(hostAlias).publicKeyDer(), host.hostSpki())
    }

    @Test
    fun aListenerPinnedToAnotherKeyIsRefusedByTheKeystoreHost() {
        val session = "host-tls-wrong-${UUID.randomUUID()}"
        val host = FfiProofHostListener.bindWithPlatformKey(
            "127.0.0.1:0",
            KeystoreHostTlsSigner(HostTlsIdentityManager(hostAlias)),
        )
        val pending = PendingHost(host, session)
        val otherKey = ListenerIdentityManager(listenerAlias).publicKeyDer()

        assertThrows(Exception::class.java) {
            listenerProveToHost(
                "127.0.0.1:${host.localPort()}",
                otherKey,
                session,
                1UL,
                KeystoreListenerProofSigner(ListenerIdentityManager(listenerAlias)),
            )
        }
        val (verified, _) = pending.finish()
        assertNull("no proof may verify through a mismatched pin", verified)
    }
}
