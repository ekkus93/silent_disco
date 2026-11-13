package com.ekkus.silentdisco

import com.ekkus.silentdisco.core.identity.HostTlsIdentityManager
import com.ekkus.silentdisco.core.identity.KeystoreListenerProofSigner
import com.ekkus.silentdisco.core.identity.ListenerIdentityManager
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportHandle
import com.ekkus.silentdisco.core.uniffi.listenerDeviceIdForKey
import java.security.KeyStore
import java.util.UUID

/**
 * Real v4 listeners for instrumented tests against the production
 * [com.ekkus.silentdisco.core.rust.HostTransportController], which binds
 * secure transport (QUIC) with this device's Keystore host TLS key (phase 4).
 *
 * A v4 host admits a listener only under the device ID its key is bound to,
 * so each test name ("denied-device", "paused-listener", ...) gets its own
 * Keystore key: [id] is the ID the host sees for that name, and [connect]
 * joins with that key, pinned to the host's key. Joining again under the same
 * name reuses the key, as a real phone's rejoin does. [close] deletes the
 * keys this instance created.
 */
internal class V4TestListeners : AutoCloseable {
    private val run = UUID.randomUUID().toString()
    private val identities = mutableMapOf<String, ListenerIdentityManager>()
    private val ids = mutableMapOf<String, String>()

    private fun alias(name: String) = "silent-disco-test-listener-$name-$run"

    private fun identity(name: String): ListenerIdentityManager =
        identities.getOrPut(name) { ListenerIdentityManager(alias(name)) }

    /** The device ID a v4 host admits [name] under. */
    fun id(name: String): String =
        ids.getOrPut(name) { listenerDeviceIdForKey(identity(name).publicKeyDer()) }

    /** Joins [endpoint] over QUIC as [name], pinned to this device's host TLS key. */
    fun connect(endpoint: String, name: String): FfiListenerTransportHandle =
        FfiListenerTransportHandle.connectQuic(
            endpoint,
            System.currentTimeMillis().toULong(),
            id(name),
            "127.0.0.1",
            HostTlsIdentityManager().publicKeyDer(),
            KeystoreListenerProofSigner(identity(name)),
        )

    override fun close() {
        val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        identities.keys.forEach { name -> runCatching { keyStore.deleteEntry(alias(name)) } }
    }
}

/** A v4 endpoint payload for a host bound on [ports]`[0]` (QUIC uses one port). */
internal fun v4Endpoint(ports: List<Int>, sessionId: String, inviteCodeRequired: Boolean = false): String =
    """{"hostAddress":"127.0.0.1","controlPort":${ports[0]},"syncPort":${ports[1]},""" +
        """"audioPort":${ports[2]},"sessionId":"$sessionId","protocolVersion":4,""" +
        """"inviteCodeRequired":$inviteCodeRequired,"expiresAtMs":null}"""
