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
        val heartbeats = mutableListOf<Job>()
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

            fun keepAlive(listener: FfiListenerTransportHandle) {
                heartbeats += launch(Dispatchers.IO) {
                    var correlation = 1uL
                    while (isActive) {
                        listener.sendSyncRequest(
                            correlation++,
                            (System.nanoTime() / 1_000_000L).toULong(),
                        )
                        delay(2_000)
                    }
                }
            }

            val oldEndpoint = bindSession("retired-session-${System.nanoTime()}")
            // A rebind must discard the old session's *paused playback cache*
            // as well as the queued join events. The replacement can reuse
            // packet sequence zero without inheriting the previous boundary.
            val oldStart = SystemClock.elapsedRealtime() + 120_000L
            host.broadcastStreamStart(
                "old-stream", oldStart, 48_000, 2, 240,
                "old-entry", "Retired session song",
            )
            host.broadcastTrackBoundary("old-stream", 480, "old-next", "Retired next song")
            host.broadcastPause("old-stream", oldStart + 100L)
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
            // Reuse the *retired session's device ID* while its old TCP
            // socket is still open. Session rebind must not confuse the two
            // identities or let the old reader retire the replacement route.
            join(newEndpoint, "old-first")

            // The native handshake also reports PeerAccepted. Ignore only
            // non-join handshakes: NEVER filter by device ID, because doing
            // so would hide queued old-session JoinRequestReceived events.
            val delivered = withTimeout(10_000) {
                host.events.first { it is FfiHostTransportEvent.JoinRequestReceived }
            }
            assertThat(delivered).isInstanceOf(FfiHostTransportEvent.JoinRequestReceived::class.java)
            assertThat((delivered as FfiHostTransportEvent.JoinRequestReceived).deviceId)
                .isEqualTo("old-first")

            val newListener = listeners.last()
            assertThat(host.sendJoinApproval(delivered.deviceId, false)?.successfulPeers)
                .isEqualTo(1u)
            assertThat(awaitReboundListenerEvent(newListener) {
                it is FfiListenerTransportEvent.JoinApproved
            }).isInstanceOf(FfiListenerTransportEvent.JoinApproved::class.java)
            host.authorizeListener(delivered.deviceId, delivered.syncPort, delivered.audioPort)
            keepAlive(newListener)

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

            // Pause, Resume and Stop/replay on the *replacement* socket must
            // use only its generation. This exercises the production control
            // cache and authenticated UDP gate after the old poller exits.
            val pauseAt = newStart + 20L
            assertThat(host.broadcastPause("new-stream", pauseAt)?.successfulPeers).isEqualTo(1u)
            assertThat(awaitReboundListenerEvent(newListener) {
                it is FfiListenerTransportEvent.Paused
            }).isEqualTo(FfiListenerTransportEvent.Paused("new-stream", pauseAt.toULong()))
            assertThat(host.broadcastAudio(
                "new-stream", 1, 48_000, 2, 240, 240, newStart + 5L,
                ByteArray(240 * 2 * 2) { 0x46.toByte() },
            )?.successfulPeers).isEqualTo(0u)

            val resumedAt = newStart + 2_000L
            assertThat(host.broadcastStreamStart(
                "new-stream", resumedAt, 48_000, 2, 240,
                "new-current-entry", "Resumed session song",
            )?.successfulPeers).isEqualTo(1u)
            val resumed = awaitReboundListenerEvent(newListener) {
                it is FfiListenerTransportEvent.StreamStarted
            } as FfiListenerTransportEvent.StreamStarted
            assertThat(resumed.streamId).isEqualTo("new-stream")
            assertThat(resumed.hostStartTimeMs).isEqualTo(resumedAt.toULong())
            assertThat(resumed.currentDisplayName).isEqualTo("Resumed session song")
            val resumedPayload = ByteArray(240 * 2 * 2) { 0x46.toByte() }
            assertThat(host.broadcastAudio(
                "new-stream", 1, 48_000, 2, 240, 240, resumedAt + 5L, resumedPayload,
            )?.successfulPeers).isEqualTo(1u)
            val resumedAudio = awaitReboundListenerEvent(newListener) {
                it is FfiListenerTransportEvent.AudioReceived
            } as FfiListenerTransportEvent.AudioReceived
            assertThat(resumedAudio.sequence).isEqualTo(1uL)
            assertThat(resumedAudio.firstSampleIndex).isEqualTo(240uL)
            assertThat(resumedAudio.payload.toList())
                .containsExactlyElementsIn(resumedPayload.toList()).inOrder()

            // A second real TCP/UDP client joins *after* the replacement
            // session resumed. Its replay must contain only the resumed
            // title, current anchor, and pending boundary, never the
            // retired session's paused song or the replacement's old Pause.
            join(newEndpoint, "new-late")
            val lateRequest = withTimeout(10_000) {
                host.events.first { it is FfiHostTransportEvent.JoinRequestReceived }
            } as FfiHostTransportEvent.JoinRequestReceived
            assertThat(lateRequest.deviceId).isEqualTo("new-late")
            val lateListener = listeners.last()
            assertThat(host.sendJoinApproval(lateRequest.deviceId, false)?.successfulPeers)
                .isEqualTo(1u)
            assertThat(awaitReboundListenerEvent(lateListener) {
                it is FfiListenerTransportEvent.JoinApproved
            }).isInstanceOf(FfiListenerTransportEvent.JoinApproved::class.java)
            host.authorizeListener(lateRequest.deviceId, lateRequest.syncPort, lateRequest.audioPort)
            keepAlive(lateListener)
            val lateStart = awaitReboundListenerEvent(lateListener) {
                it is FfiListenerTransportEvent.StreamStarted
            } as FfiListenerTransportEvent.StreamStarted
            assertThat(lateStart.streamId).isEqualTo("new-stream")
            assertThat(lateStart.hostStartTimeMs).isEqualTo(resumedAt.toULong())
            assertThat(lateStart.currentEntryId).isEqualTo("new-current-entry")
            assertThat(lateStart.currentDisplayName).isEqualTo("Resumed session song")
            val lateBoundary = awaitReboundListenerEvent(lateListener) {
                it is FfiListenerTransportEvent.TrackBoundary
            } as FfiListenerTransportEvent.TrackBoundary
            assertThat(lateBoundary.streamId).isEqualTo("new-stream")
            assertThat(lateBoundary.firstSampleIndex).isEqualTo(480uL)
            assertThat(lateBoundary.entryId).isEqualTo("new-next-entry")

            // Both authorized sockets must receive identical UDP frames
            // after the late join, without reopening or rebinding the host.
            val sharedPayload = ByteArray(240 * 2 * 2) { 0x4b.toByte() }
            assertThat(host.broadcastAudio(
                "new-stream", 2, 48_000, 2, 240, 480, resumedAt + 10L, sharedPayload,
            )?.successfulPeers).isEqualTo(2u)
            val originalShared = awaitReboundListenerEvent(newListener) {
                it is FfiListenerTransportEvent.AudioReceived
            } as FfiListenerTransportEvent.AudioReceived
            val lateShared = awaitReboundListenerEvent(lateListener) {
                it is FfiListenerTransportEvent.AudioReceived
            } as FfiListenerTransportEvent.AudioReceived
            for (packet in listOf(originalShared, lateShared)) {
                assertThat(packet.streamId).isEqualTo("new-stream")
                assertThat(packet.sequence).isEqualTo(2uL)
                assertThat(packet.firstSampleIndex).isEqualTo(480uL)
                assertThat(packet.hostPresentationTimeMs).isEqualTo((resumedAt + 10L).toULong())
                assertThat(packet.payload.toList())
                    .containsExactlyElementsIn(sharedPayload.toList()).inOrder()
            }

            assertThat(host.broadcastStop(
                "new-stream", SystemClock.elapsedRealtime(),
            )?.successfulPeers).isEqualTo(2u)
            assertThat(awaitReboundListenerEvent(newListener) {
                it is FfiListenerTransportEvent.Stopped
            }).isInstanceOf(FfiListenerTransportEvent.Stopped::class.java)
            assertThat(awaitReboundListenerEvent(lateListener) {
                it is FfiListenerTransportEvent.Stopped
            }).isInstanceOf(FfiListenerTransportEvent.Stopped::class.java)
            // A new Play may reuse the stream ID, but the old title,
            // paused state, and pending boundary must never be resurrected.
            val replayAt = maxOf(resumedAt + 60_000L, SystemClock.elapsedRealtime() + 600_000L)
            host.broadcastStreamStart(
                "new-stream", replayAt, 48_000, 2, 240,
                "replay-entry", "Replayed session song",
            )
            host.broadcastTrackBoundary("new-stream", 480, "replay-next", "Replay next song")
            val lateReplayStart = awaitReboundListenerEvent(lateListener) {
                it is FfiListenerTransportEvent.StreamStarted
            } as FfiListenerTransportEvent.StreamStarted
            assertThat(lateReplayStart.hostStartTimeMs).isEqualTo(replayAt.toULong())
            assertThat(lateReplayStart.currentEntryId).isEqualTo("replay-entry")
            assertThat(lateReplayStart.currentDisplayName).isEqualTo("Replayed session song")
            val replayStart = awaitReboundListenerEvent(newListener) {
                it is FfiListenerTransportEvent.StreamStarted
            } as FfiListenerTransportEvent.StreamStarted
            assertThat(replayStart.hostStartTimeMs).isEqualTo(replayAt.toULong())
            assertThat(replayStart.currentEntryId).isEqualTo("replay-entry")
            assertThat(replayStart.currentDisplayName).isEqualTo("Replayed session song")
            val replayBoundary = awaitReboundListenerEvent(newListener) {
                it is FfiListenerTransportEvent.TrackBoundary
            } as FfiListenerTransportEvent.TrackBoundary
            assertThat(replayBoundary.entryId).isEqualTo("replay-next")
            assertThat(replayBoundary.firstSampleIndex).isEqualTo(480uL)
            val lateReplayBoundary = awaitReboundListenerEvent(lateListener) {
                it is FfiListenerTransportEvent.TrackBoundary
            } as FfiListenerTransportEvent.TrackBoundary
            assertThat(lateReplayBoundary.entryId).isEqualTo("replay-next")
            assertThat(lateReplayBoundary.firstSampleIndex).isEqualTo(480uL)
            val replayPayload = ByteArray(240 * 2 * 2) { 0x57.toByte() }
            assertThat(host.broadcastAudio(
                "new-stream", 0, 48_000, 2, 240, 0, replayAt, replayPayload,
            )?.successfulPeers).isEqualTo(2u)
            val replayAudio = awaitReboundListenerEvent(newListener) {
                it is FfiListenerTransportEvent.AudioReceived
            } as FfiListenerTransportEvent.AudioReceived
            assertThat(replayAudio.sequence).isEqualTo(0uL)
            assertThat(replayAudio.firstSampleIndex).isEqualTo(0uL)
            assertThat(replayAudio.hostPresentationTimeMs).isEqualTo(replayAt.toULong())
            assertThat(replayAudio.payload.toList())
                .containsExactlyElementsIn(replayPayload.toList()).inOrder()
            val lateReplayAudio = awaitReboundListenerEvent(lateListener) {
                it is FfiListenerTransportEvent.AudioReceived
            } as FfiListenerTransportEvent.AudioReceived
            assertThat(lateReplayAudio.sequence).isEqualTo(0uL)
            assertThat(lateReplayAudio.firstSampleIndex).isEqualTo(0uL)
            assertThat(lateReplayAudio.hostPresentationTimeMs).isEqualTo(replayAt.toULong())
            assertThat(lateReplayAudio.payload.toList())
                .containsExactlyElementsIn(replayPayload.toList()).inOrder()
        } finally {
            heartbeats.forEach { it.cancelAndJoin() }
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
