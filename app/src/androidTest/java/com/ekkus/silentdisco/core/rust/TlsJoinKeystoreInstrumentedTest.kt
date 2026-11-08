package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.app.LISTENER_DISCOVERY_PROTOCOL_VERSION
import com.ekkus.silentdisco.core.identity.HostTlsIdentityManager
import com.ekkus.silentdisco.core.identity.KeystoreHostTlsSigner
import com.ekkus.silentdisco.core.identity.KeystoreListenerProofSigner
import com.ekkus.silentdisco.core.identity.ListenerIdentityManager
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportHandle
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportHandle
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportException
import com.ekkus.silentdisco.core.uniffi.listenerDeviceIdForKey
import java.net.ServerSocket
import java.security.KeyStore
import java.util.UUID
import org.junit.After
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A complete TLS join on one device: a host whose TLS key stays in Keystore
 * accepts a listener pinned to that key, the listener's Keystore proof is
 * verified, its join reaches the host, and the approval returns on the same
 * channel. A listener pinned to any other key never connects.
 */
class TlsJoinKeystoreInstrumentedTest {
    private val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
    private val hostAlias = "silent-disco-tls-join-host-${UUID.randomUUID()}"
    private val listenerAlias = "silent-disco-tls-join-listener-${UUID.randomUUID()}"
    private val session = "tls-join-${UUID.randomUUID()}"
    // A pinned join must claim the device ID its listener key is bound to.
    private fun deviceId(): String = listenerDeviceIdForKey(ListenerIdentityManager(listenerAlias).publicKeyDer())

    @After
    fun deleteKeys() {
        runCatching { keyStore.deleteEntry(hostAlias) }
        runCatching { keyStore.deleteEntry(listenerAlias) }
    }

    private fun freePorts(): IntArray = IntArray(3) { ServerSocket(0).use { it.localPort } }

    private fun endpointPayload(ports: IntArray): String =
        """{"hostAddress":"127.0.0.1","controlPort":${ports[0]},"syncPort":${ports[1]},""" +
            """"audioPort":${ports[2]},"sessionId":"$session","protocolVersion":$LISTENER_DISCOVERY_PROTOCOL_VERSION,""" +
            """"inviteCodeRequired":false,"expiresAtMs":null}"""

    @Test
    fun aPinnedListenerJoinsAKeystoreHostOverTlsAndIsApproved() {
        val ports = freePorts()
        val host = FfiHostTransportHandle.bindSecureWithHostClock(
            "127.0.0.1", ports[0].toUShort(), ports[1].toUShort(), ports[2].toUShort(),
            session, 0UL,
            KeystoreHostTlsSigner(HostTlsIdentityManager(hostAlias)),
        )
        val pin = HostTlsIdentityManager(hostAlias).publicKeyDer()
        val listener = FfiListenerTransportHandle.connectSecure(
            endpointPayload(ports), 0UL, deviceId(), "127.0.0.1", pin,
            KeystoreListenerProofSigner(ListenerIdentityManager(listenerAlias)),
        )
        try {
            listener.sendJoinRequest("Pinned listener", null)
            assertTrue("the host must receive the TLS join", awaitHostJoin(host))
            host.sendJoinApproval(deviceId(), false)
            assertTrue("the approval must return over the same TLS channel", awaitApproval(listener))
        } finally {
            listener.shutdown()
            host.shutdown()
        }
    }

    @Test
    fun aListenerPinnedToAnotherKeyNeverConnectsToTheKeystoreHost() {
        val ports = freePorts()
        val host = FfiHostTransportHandle.bindSecureWithHostClock(
            "127.0.0.1", ports[0].toUShort(), ports[1].toUShort(), ports[2].toUShort(),
            session, 0UL,
            KeystoreHostTlsSigner(HostTlsIdentityManager(hostAlias)),
        )
        val wrongPin = ListenerIdentityManager(listenerAlias).publicKeyDer()
        try {
            assertThrows(FfiListenerTransportException::class.java) {
                FfiListenerTransportHandle.connectSecure(
                    endpointPayload(ports), 0UL, deviceId(), "127.0.0.1", wrongPin,
                    KeystoreListenerProofSigner(ListenerIdentityManager(listenerAlias)),
                )
            }
        } finally {
            host.shutdown()
        }
    }

    private fun awaitHostJoin(host: FfiHostTransportHandle): Boolean {
        val deadline = System.currentTimeMillis() + 15_000
        while (System.currentTimeMillis() < deadline) {
            val event = host.pollEvent(200UL)
            if (event is FfiHostTransportEvent.JoinRequestReceived && event.deviceId == deviceId()) return true
        }
        return false
    }

    private fun awaitApproval(listener: FfiListenerTransportHandle): Boolean {
        val deadline = System.currentTimeMillis() + 15_000
        while (System.currentTimeMillis() < deadline) {
            val event = listener.pollEvent(200UL)
            if (event is FfiListenerTransportEvent.JoinApproved) return true
        }
        return false
    }
}
