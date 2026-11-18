package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.app.controller.SECURE_TRANSPORT_ENDPOINT_VERSION
import com.ekkus.silentdisco.core.identity.HostTlsIdentityManager
import com.ekkus.silentdisco.core.identity.KeystoreHostTlsSigner
import com.ekkus.silentdisco.core.identity.KeystoreListenerProofSigner
import com.ekkus.silentdisco.core.identity.ListenerIdentityManager
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportHandle
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportHandle
import com.ekkus.silentdisco.core.uniffi.listenerDeviceIdForKey
import com.google.common.truth.Truth.assertThat
import java.net.ServerSocket
import java.security.KeyStore
import java.util.UUID
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Test

/**
 * Remediation TODO §1 (reconnect), on the device: the production
 * [ListenerTransportController] joins a Keystore host over v4 QUIC, drops,
 * and rejoins with the same Keystore key. The host sees the same key-bound
 * device ID both times, the rejoin is approved on the new connection, and a
 * stream started after the rejoin reaches the listener. Only the listener's
 * key alias is test-only; the controller's generation gate and event queue
 * are production code.
 */
class ListenerReconnectKeystoreInstrumentedTest {
    private val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
    private val hostAlias = "silent-disco-reconnect-host-${UUID.randomUUID()}"
    private val listenerAlias = "silent-disco-reconnect-listener-${UUID.randomUUID()}"
    private val session = "reconnect-${UUID.randomUUID()}"
    private val deviceId by lazy { listenerDeviceIdForKey(ListenerIdentityManager(listenerAlias).publicKeyDer()) }

    @After
    fun deleteKeys() {
        listOf(hostAlias, listenerAlias).forEach { if (keyStore.containsAlias(it)) keyStore.deleteEntry(it) }
    }

    private fun endpointPayload(port: Int): String =
        """{"hostAddress":"127.0.0.1","controlPort":$port,"syncPort":$port,""" +
            """"audioPort":$port,"sessionId":"$session","protocolVersion":$SECURE_TRANSPORT_ENDPOINT_VERSION,""" +
            """"inviteCodeRequired":false,"expiresAtMs":null}"""

    private fun awaitHost(host: FfiHostTransportHandle, pick: (FfiHostTransportEvent) -> Boolean): FfiHostTransportEvent {
        val deadline = System.currentTimeMillis() + 15_000
        while (System.currentTimeMillis() < deadline) {
            val event = host.pollEvent(200UL) ?: continue
            if (pick(event)) return event
        }
        throw AssertionError("expected host event did not arrive")
    }

    @Test
    fun aListenerThatDropsRejoinsWithTheSameKeyAndReceivesTheStreamOnTheNewConnection() = runBlocking {
        val port = ServerSocket(0).use { it.localPort }
        val host = FfiHostTransportHandle.bindQuicWithHostClock(
            "127.0.0.1", port.toUShort(), session, 0UL,
            KeystoreHostTlsSigner(HostTlsIdentityManager(hostAlias)),
        )
        val pin = HostTlsIdentityManager(hostAlias).publicKeyDer()
        val controller = ListenerTransportController(
            openPinnedHandle = { endpoint, now, address, hostKey ->
                FfiListenerTransportHandle.connectSecure(
                    endpoint, now, deviceId, address, hostKey,
                    KeystoreListenerProofSigner(ListenerIdentityManager(listenerAlias)),
                )
            },
        )
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        val events = Channel<FfiListenerTransportEvent>(Channel.UNLIMITED)
        scope.launch { controller.collectCurrentEvents { events.send(it) } }
        suspend fun awaitListener(pick: (FfiListenerTransportEvent) -> Boolean) = withTimeout(15_000) {
            while (true) if (pick(events.receive())) break
        }
        suspend fun joinAndApprove() {
            controller.sendJoinRequest("Reconnecting listener", null)
            val join = awaitHost(host) {
                it is FfiHostTransportEvent.JoinRequestReceived && it.deviceId == deviceId
            } as FfiHostTransportEvent.JoinRequestReceived
            host.sendJoinApproval(deviceId, false)
            host.authorizeListener(deviceId, join.syncPort, join.audioPort)
            awaitListener { it is FfiListenerTransportEvent.JoinApproved }
        }
        try {
            controller.connect(scope, endpointPayload(port), deviceId, "127.0.0.1", pin)
            joinAndApprove()

            // The network drops: the old connection goes away.
            controller.disconnect("network dropped")
            awaitHost(host) {
                (it is FfiHostTransportEvent.PeerDisconnectedAt && it.listenerId == deviceId) ||
                    (it is FfiHostTransportEvent.PeerDisconnected && it.listenerId == deviceId)
            }

            // Rejoin with the same Keystore key: the host sees the same device ID.
            controller.connect(scope, endpointPayload(port), deviceId, "127.0.0.1", pin)
            joinAndApprove()

            host.broadcastStreamStart("stream-after-rejoin", 5_000UL, 48_000U, 2U.toUShort(), 240U, null, null)
            var started: String? = null
            awaitListener { event ->
                (event is FfiListenerTransportEvent.StreamStarted).also { if (it) started = (event as FfiListenerTransportEvent.StreamStarted).streamId }
            }
            assertThat(started).isEqualTo("stream-after-rejoin")
        } finally {
            controller.close()
            scope.cancel()
            host.shutdown()
        }
    }
}
