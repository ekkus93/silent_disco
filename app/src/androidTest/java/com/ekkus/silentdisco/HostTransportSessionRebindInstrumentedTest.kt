package com.ekkus.silentdisco

import android.os.SystemClock
import com.ekkus.silentdisco.core.rust.HostTransportController
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportHandle
import com.google.common.truth.Truth.assertThat
import java.net.ServerSocket
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
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
        var heartbeat: Job? = null
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
            // Seed the NEW host's authoritative cache before the listener
            // joins. Rebind must not reuse the retired host's socket routes,
            // buffered admission events, or playback-control generation.
            val newStart = SystemClock.elapsedRealtime() + 600_000L
            host.broadcastStreamStart(
                "new-stream", newStart, 48_000, 2, 240,
                "new-current-entry", "New session song",
            )
            host.broadcastTrackBoundary("new-stream", 480, "new-next-entry", "New next song")
            join(newEndpoint, "new-current")

            // The native handshake also reports PeerAccepted. Ignore only
            // non-join handshakes: NEVER filter by device ID, because doing
            // so would hide queued old-session JoinRequestReceived events.
            val delivered = withTimeout(10_000) {
                host.events.first { it is FfiHostTransportEvent.JoinRequestReceived }
            }
            assertThat(delivered).isInstanceOf(FfiHostTransportEvent.JoinRequestReceived::class.java)
            assertThat((delivered as FfiHostTransportEvent.JoinRequestReceived).deviceId)
                .isEqualTo("new-current")

            val newListener = listeners.last()
            assertThat(host.sendJoinApproval(delivered.deviceId, false)?.successfulPeers)
                .isEqualTo(1u)
            assertThat(awaitReboundListenerEvent(newListener) {
                it is FfiListenerTransportEvent.JoinApproved
            }).isInstanceOf(FfiListenerTransportEvent.JoinApproved::class.java)
            host.authorizeListener(delivered.deviceId, delivered.syncPort, delivered.audioPort)
            heartbeat = launch(Dispatchers.IO) {
                var correlation = 1uL
                while (isActive) {
                    newListener.sendSyncRequest(
                        correlation++,
                        (System.nanoTime() / 1_000_000L).toULong(),
                    )
                    delay(2_000)
                }
            }

            val started = awaitReboundListenerEvent(newListener) {
                it is FfiListenerTransportEvent.StreamStarted
            } as FfiListenerTransportEvent.StreamStarted
            assertThat(started.streamId).isEqualTo("new-stream")
            assertThat(started.hostStartTimeMs).isEqualTo(newStart.toULong())
            assertThat(started.currentEntryId).isEqualTo("new-current-entry")
            assertThat(started.currentDisplayName).isEqualTo("New session song")
            val boundary = awaitReboundListenerEvent(newListener) {
                it is FfiListenerTransportEvent.TrackBoundary
            } as FfiListenerTransportEvent.TrackBoundary
            assertThat(boundary.streamId).isEqualTo("new-stream")
            assertThat(boundary.firstSampleIndex).isEqualTo(480uL)
            assertThat(boundary.entryId).isEqualTo("new-next-entry")

            // Retire the old sockets after the new listener is admitted.
            // Their delayed shutdown must not disturb the new route or
            // cause the new listener to receive stale-session PCM.
            listeners.dropLast(1).forEach { runCatching { it.shutdown() } }
            val payload = ByteArray(240 * 2 * 2) { 0x35.toByte() }
            assertThat(host.broadcastAudio(
                "new-stream", 0, 48_000, 2, 240, 0, newStart, payload,
            )?.successfulPeers).isEqualTo(1u)
            val audio = awaitReboundListenerEvent(newListener) {
                it is FfiListenerTransportEvent.AudioReceived
            } as FfiListenerTransportEvent.AudioReceived
            assertThat(audio.streamId).isEqualTo("new-stream")
            assertThat(audio.sequence).isEqualTo(0uL)
            assertThat(audio.firstSampleIndex).isEqualTo(0uL)
            assertThat(audio.hostPresentationTimeMs).isEqualTo(newStart.toULong())
            assertThat(audio.payload.toList()).containsExactlyElementsIn(payload.toList()).inOrder()
        } finally {
            heartbeat?.cancelAndJoin()
            listeners.forEach { runCatching { it.shutdown() } }
            host.close()
        }
    }
}

private suspend fun awaitReboundListenerEvent(
    listener: FfiListenerTransportHandle,
    matches: (FfiListenerTransportEvent) -> Boolean,
): FfiListenerTransportEvent = withTimeout(10_000) {
    while (true) {
        val event = listener.pollEvent(500uL)
        if (event != null && matches(event)) return@withTimeout event
    }
    error("timed out waiting for rebound listener event")
}
