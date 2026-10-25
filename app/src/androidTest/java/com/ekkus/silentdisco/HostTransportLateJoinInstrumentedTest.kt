package com.ekkus.silentdisco

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
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Test

class HostTransportLateJoinInstrumentedTest {
    @Test
    fun approvedListenerReceivesTargetedCurrentTrackAndPendingBoundary() = runBlocking<Unit> {
        val portSockets = List(3) { ServerSocket(0) }
        val ports = portSockets.map(ServerSocket::getLocalPort)
        portSockets.forEach(ServerSocket::close)
        val sessionId = "android-late-join-${System.nanoTime()}"
        val host = HostTransportController()
        var listener: FfiListenerTransportHandle? = null
        var syncKeepAlive: Job? = null
        try {
            host.bind(
                scope = this,
                localAddress = "127.0.0.1",
                controlPort = ports[0],
                syncPort = ports[1],
                audioPort = ports[2],
                sessionId = sessionId,
            )
            host.broadcastStreamStart(
                streamId = "active-stream",
                // The production socket clock starts at bind. Leave enough time for
                // a slow emulator to authorize the listener before the first boundary.
                hostStartTimeMs = 60_000,
                sampleRate = 48_000,
                channels = 2,
                samplesPerPacket = 240,
                currentEntryId = "entry-current",
                currentDisplayName = "Current song",
            )
            host.broadcastTrackBoundary(
                streamId = "active-stream",
                firstSampleIndex = 24_000,
                entryId = "entry-next",
                displayName = "Next song",
            )
            val endpoint = """
                {"hostAddress":"127.0.0.1","controlPort":${ports[0]},"syncPort":${ports[1]},
                "audioPort":${ports[2]},"sessionId":"$sessionId","protocolVersion":3,
                "inviteCodeRequired":false,"expiresAtMs":null}
            """.trimIndent().replace("\n", "")
            listener = FfiListenerTransportHandle.connect(
                endpoint,
                System.currentTimeMillis().toULong(),
                "android-late-listener",
                "127.0.0.1",
            )
            listener.sendJoinRequest("Late Listener", null)
            val join = withTimeout(5_000) {
                host.events.first { it is FfiHostTransportEvent.JoinRequestReceived }
            } as FfiHostTransportEvent.JoinRequestReceived
            host.sendJoinApproval(join.deviceId, false)
            assertThat(awaitListenerEvent(listener) { it is FfiListenerTransportEvent.JoinApproved }).isInstanceOf(
                FfiListenerTransportEvent.JoinApproved::class.java,
            )
            host.authorizeListener(join.deviceId, join.syncPort, join.audioPort)
            // The production listener probes every two seconds. Without probes, the
            // host correctly evicts an otherwise silent streaming peer after eight
            // seconds; a slow emulator can exceed that window during assertions.
            // Keep the real transport alive rather than weakening its liveness gate.
            val activeListener = requireNotNull(listener)
            syncKeepAlive = launch(Dispatchers.IO) {
                var correlationId = 1uL
                while (isActive) {
                    activeListener.sendSyncRequest(
                        correlationId++,
                        (System.nanoTime() / 1_000_000L).toULong(),
                    )
                    delay(2_000L)
                }
            }

            assertThat(awaitListenerEvent(listener) { it is FfiListenerTransportEvent.StreamStarted }).isEqualTo(
                FfiListenerTransportEvent.StreamStarted(
                    streamId = "active-stream",
                    hostStartTimeMs = 60_000uL,
                    sampleRate = 48_000u,
                    channels = 2u,
                    samplesPerPacket = 240u,
                    currentEntryId = "entry-current",
                    currentDisplayName = "Current song",
                ),
            )

            assertThat(awaitListenerEvent(listener) { it is FfiListenerTransportEvent.TrackBoundary }).isEqualTo(
                FfiListenerTransportEvent.TrackBoundary(
                    streamId = "active-stream",
                    firstSampleIndex = 24_000uL,
                    entryId = "entry-next",
                    displayName = "Next song",
                ),
            )

            host.broadcastAudio(
                streamId = "active-stream",
                sequence = 100,
                sampleRate = 48_000,
                channels = 2,
                samplesPerPacket = 240,
                firstSampleIndex = 24_000,
                hostPresentationTimeMs = 60_500,
                payload = ByteArray(240 * 2 * 2),
            )
            val audio = awaitListenerEvent(listener) {
                it is FfiListenerTransportEvent.AudioReceived
            } as FfiListenerTransportEvent.AudioReceived
            assertThat(audio.streamId).isEqualTo("active-stream")
            assertThat(audio.sequence).isEqualTo(100uL)
            assertThat(audio.sampleRate).isEqualTo(48_000u)
            assertThat(audio.channels).isEqualTo(2u.toUShort())
            assertThat(audio.samplesPerPacket).isEqualTo(240u)
            assertThat(audio.firstSampleIndex).isEqualTo(24_000uL)
            assertThat(audio.hostPresentationTimeMs).isEqualTo(60_500uL)
            assertThat(audio.payload.toList()).containsExactlyElementsIn(ByteArray(240 * 2 * 2).toList())
        } finally {
            syncKeepAlive?.cancelAndJoin()
            listener?.shutdown()
            host.close()
        }
    }

    @Test
    fun listenerJoiningAfterFirstBoundaryReceivesAudibleTrackAndOnlyUpcomingBoundary() = runBlocking<Unit> {
        val sockets = List(3) { ServerSocket(0) }
        val ports = sockets.map(ServerSocket::getLocalPort)
        sockets.forEach(ServerSocket::close)
        val sessionId = "android-past-boundary-${System.nanoTime()}"
        val host = HostTransportController()
        var listener: FfiListenerTransportHandle? = null
        try {
            host.bind(this, "127.0.0.1", ports[0], ports[1], ports[2], sessionId)
            // The production transport clock begins at bind. The first
            // boundary is already audible, while the second is far enough
            // ahead to remain pending even on a loaded hosted emulator.
            host.broadcastStreamStart(
                "active-stream", 0, 48_000, 2, 240, "entry-first", "First song",
            )
            host.broadcastTrackBoundary("active-stream", 240, "entry-second", "Second song")
            host.broadcastTrackBoundary("active-stream", 2_400_000, "entry-third", "Third song")
            delay(50L)

            val endpoint = """
                {"hostAddress":"127.0.0.1","controlPort":${ports[0]},"syncPort":${ports[1]},
                "audioPort":${ports[2]},"sessionId":"$sessionId","protocolVersion":3,
                "inviteCodeRequired":false,"expiresAtMs":null}
            """.trimIndent().replace("\n", "")
            listener = FfiListenerTransportHandle.connect(
                endpoint,
                System.currentTimeMillis().toULong(),
                "android-past-boundary-listener",
                "127.0.0.1",
            )
            listener.sendJoinRequest("Late Listener", null)
            val join = withTimeout(5_000) {
                host.events.first { it is FfiHostTransportEvent.JoinRequestReceived }
            } as FfiHostTransportEvent.JoinRequestReceived
            host.sendJoinApproval(join.deviceId, false)
            assertThat(awaitListenerEvent(listener) { it is FfiListenerTransportEvent.JoinApproved })
                .isInstanceOf(FfiListenerTransportEvent.JoinApproved::class.java)
            host.authorizeListener(join.deviceId, join.syncPort, join.audioPort)

            assertThat(awaitListenerEvent(listener) { it is FfiListenerTransportEvent.StreamStarted })
                .isEqualTo(
                    FfiListenerTransportEvent.StreamStarted(
                        "active-stream", 0uL, 48_000u, 2u, 240u,
                        "entry-second", "Second song",
                    ),
                )
            // The already-audible first boundary must not be replayed;
            // only the future third entry remains in the reconnect cache.
            assertThat(awaitListenerEvent(listener) { it is FfiListenerTransportEvent.TrackBoundary })
                .isEqualTo(
                    FfiListenerTransportEvent.TrackBoundary(
                        "active-stream", 2_400_000uL, "entry-third", "Third song",
                    ),
                )
        } finally {
            listener?.shutdown()
            host.close()
        }
    }
}

private suspend fun awaitListenerEvent(
    listener: FfiListenerTransportHandle,
    matches: (FfiListenerTransportEvent) -> Boolean,
): FfiListenerTransportEvent = withTimeout(5_000) {
    while (true) {
        val event = listener.pollEvent(500uL)
        if (event != null && matches(event)) return@withTimeout event
    }
    error("timed out waiting for listener event")
}
