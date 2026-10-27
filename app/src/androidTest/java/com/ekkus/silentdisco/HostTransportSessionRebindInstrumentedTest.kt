package com.ekkus.silentdisco

import com.ekkus.silentdisco.core.rust.HostTransportController
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportHandle
import com.google.common.truth.Truth.assertThat
import java.net.ServerSocket
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Test

/**
 * Production TCP host rebind: the event channel deliberately has no collector
 * while a previous session produces admission events. The new session must
 * never receive queued join/disconnect/error events from the closed socket.
 */
class HostTransportSessionRebindInstrumentedTest {
    @Test
    fun sequentialHostsDiscardBufferedOldSessionJoinEvents() = runBlocking<Unit> {
        val host = HostTransportController()
        val listeners = mutableListOf<FfiListenerTransportHandle>()
        try {
            suspend fun bindSession(sessionId: String): String {
                val reserved = List(3) { ServerSocket(0) }
                val ports = reserved.map(ServerSocket::getLocalPort)
                reserved.forEach(ServerSocket::close)
                host.bind(
                    scope = this@runBlocking,
                    localAddress = "127.0.0.1",
                    controlPort = ports[0],
                    syncPort = ports[1],
                    audioPort = ports[2],
                    sessionId = sessionId,
                )
                return """
                    {"hostAddress":"127.0.0.1","controlPort":${ports[0]},"syncPort":${ports[1]},
                    "audioPort":${ports[2]},"sessionId":"$sessionId","protocolVersion":3,
                    "inviteCodeRequired":false,"expiresAtMs":null}
                """.trimIndent().replace("\n", "")
            }

            fun join(endpoint: String, device: String) {
                val listener = FfiListenerTransportHandle.connect(
                    endpoint,
                    System.currentTimeMillis().toULong(),
                    device,
                    "127.0.0.1",
                )
                listeners += listener
                listener.sendJoinRequest(device, null)
            }

            val oldEndpoint = bindSession("retired-session-${System.nanoTime()}")
            join(oldEndpoint, "old-first")
            // Observe the first event to establish that the native TCP poller
            // is actually running before leaving later events queued.
            val first = withTimeout(10_000) {
                host.events.first { it is FfiHostTransportEvent.JoinRequestReceived }
            } as FfiHostTransportEvent.JoinRequestReceived
            assertThat(first.deviceId).isEqualTo("old-first")

            join(oldEndpoint, "old-queued-1")
            join(oldEndpoint, "old-queued-2")
            // Neither old admission event is consumed; at least one should
            // have reached the buffer before the old host is torn down.
            delay(600)
            val newEndpoint = bindSession("replacement-session-${System.nanoTime()}")
            join(newEndpoint, "new-current")

            // first() without a device predicate is deliberate: an obsolete
            // queued event would otherwise be skipped and hide the defect.
            val delivered = withTimeout(10_000) { host.events.first() }
            assertThat(delivered).isInstanceOf(FfiHostTransportEvent.JoinRequestReceived::class.java)
            assertThat((delivered as FfiHostTransportEvent.JoinRequestReceived).deviceId)
                .isEqualTo("new-current")
        } finally {
            listeners.forEach { runCatching { it.shutdown() } }
            host.close()
        }
    }
}
