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
            // Host controls and the production socket snapshot now share
            // Android's boot-relative elapsedRealtime clock domain.
            val hostStartTimeMs = SystemClock.elapsedRealtime() + 60_000L
            host.broadcastStreamStart(
                streamId = "active-stream",
                // Leave time for a slow emulator to authorize before the boundary.
                hostStartTimeMs = hostStartTimeMs,
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
                    hostStartTimeMs = hostStartTimeMs.toULong(),
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
                hostPresentationTimeMs = hostStartTimeMs + 500L,
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
            assertThat(audio.hostPresentationTimeMs).isEqualTo((hostStartTimeMs + 500L).toULong())
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
            // This stream started one second ago on the *same* boot-relative
            // clock used by the Android host packetizer and sync responses.
            // The first boundary is audible; the second remains in the future.
            val hostStartTimeMs = SystemClock.elapsedRealtime() - 1_000L
            host.broadcastStreamStart(
                "active-stream", hostStartTimeMs, 48_000, 2, 240, "entry-first", "First song",
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
                        "active-stream", hostStartTimeMs.toULong(), 48_000u, 2u, 240u,
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

    @Test
    fun pausedLateJoinResumeAndStopReplayUseBootClockAndFreshGeneration() = runBlocking<Unit> {
        val sockets = List(3) { ServerSocket(0) }
        val ports = sockets.map(ServerSocket::getLocalPort)
        sockets.forEach(ServerSocket::close)
        val sessionId = "android-paused-join-${System.nanoTime()}"
        val host = HostTransportController()
        val listeners = mutableListOf<FfiListenerTransportHandle>()
        val syncKeepAlives = mutableListOf<Job>()
        try {
            host.bind(this, "127.0.0.1", ports[0], ports[1], ports[2], sessionId)
            val startMs = SystemClock.elapsedRealtime() - 1_000L
            val pauseMs = SystemClock.elapsedRealtime() - 100L
            host.broadcastStreamStart(
                "active-stream", startMs, 48_000, 2, 240, "first", "First track",
            )
            host.broadcastTrackBoundary("active-stream", 240, "second", "Second track")
            host.broadcastTrackBoundary("active-stream", 2_400_000, "third", "Third track")
            host.broadcastPause("active-stream", pauseMs)

            val endpoint = """
                {"hostAddress":"127.0.0.1","controlPort":${ports[0]},"syncPort":${ports[1]},
                "audioPort":${ports[2]},"sessionId":"$sessionId","protocolVersion":3,
                "inviteCodeRequired":false,"expiresAtMs":null}
            """.trimIndent().replace("\n", "")

            suspend fun join(id: String): FfiListenerTransportHandle {
                val listener = FfiListenerTransportHandle.connect(
                    endpoint, System.currentTimeMillis().toULong(), id, "127.0.0.1",
                )
                listeners += listener
                listener.sendJoinRequest(id, null)
                val request = withTimeout(5_000) {
                    host.events.first { it is FfiHostTransportEvent.JoinRequestReceived }
                } as FfiHostTransportEvent.JoinRequestReceived
                host.sendJoinApproval(request.deviceId, false)
                assertThat(awaitListenerEvent(listener) { it is FfiListenerTransportEvent.JoinApproved })
                    .isInstanceOf(FfiListenerTransportEvent.JoinApproved::class.java)
                host.authorizeListener(request.deviceId, request.syncPort, request.audioPort)
                // The host expires silent peers after eight seconds. This
                // multi-stage emulator regression can exceed that interval;
                // exercise the production sync heartbeat instead of keeping
                // an otherwise disconnected listener alive artificially.
                syncKeepAlives += launch(Dispatchers.IO) {
                    var correlationId = 1uL
                    while (isActive) {
                        listener.sendSyncRequest(
                            correlationId++,
                            (System.nanoTime() / 1_000_000L).toULong(),
                        )
                        delay(2_000L)
                    }
                }
                return listener
            }

            val paused = join("paused-listener")
            assertThat(awaitListenerEvent(paused) { it is FfiListenerTransportEvent.StreamStarted })
                .isEqualTo(
                    FfiListenerTransportEvent.StreamStarted(
                        "active-stream", startMs.toULong(), 48_000u, 2u, 240u,
                        "second", "Second track",
                    ),
                )
            assertThat(awaitListenerEvent(paused) { it is FfiListenerTransportEvent.TrackBoundary })
                .isEqualTo(
                    FfiListenerTransportEvent.TrackBoundary(
                        "active-stream", 2_400_000uL, "third", "Third track",
                    ),
                )
            assertThat(awaitListenerEvent(paused) { it is FfiListenerTransportEvent.Paused })
                .isEqualTo(FfiListenerTransportEvent.Paused("active-stream", pauseMs.toULong()))

            // Disconnect the *same* admitted device while paused, then
            // reconnect through a new TCP/UDP route. The replay must restore
            // the audible second track, pending third boundary, and Pause in
            // order; neither the first track nor an obsolete boundary may
            // leak into the new socket's presentation state.
            syncKeepAlives.first().cancelAndJoin()
            host.disconnectPeer("paused-listener", "Synthetic reconnect")
            paused.shutdown()
            val returning = join("paused-listener")
            assertThat(awaitListenerEvent(returning) { it is FfiListenerTransportEvent.StreamStarted })
                .isEqualTo(
                    FfiListenerTransportEvent.StreamStarted(
                        "active-stream", startMs.toULong(), 48_000u, 2u, 240u,
                        "second", "Second track",
                    ),
                )
            assertThat(awaitListenerEvent(returning) { it is FfiListenerTransportEvent.TrackBoundary })
                .isEqualTo(
                    FfiListenerTransportEvent.TrackBoundary(
                        "active-stream", 2_400_000uL, "third", "Third track",
                    ),
                )
            assertThat(awaitListenerEvent(returning) { it is FfiListenerTransportEvent.Paused })
                .isEqualTo(FfiListenerTransportEvent.Paused("active-stream", pauseMs.toULong()))

            // Resume shifts the host timeline without reintroducing the
            // already-audible first boundary. A new socket must see the
            // current second track, future third boundary and no old Pause.
            val resumedAnchor = SystemClock.elapsedRealtime() + 60_000L
            host.broadcastStreamStart(
                "active-stream", resumedAnchor, 48_000, 2, 240, "second", "Second track",
            )
            assertThat(awaitListenerEvent(returning) { it is FfiListenerTransportEvent.StreamStarted })
                .isEqualTo(
                    FfiListenerTransportEvent.StreamStarted(
                        "active-stream", resumedAnchor.toULong(), 48_000u, 2u, 240u,
                        "second", "Second track",
                    ),
                )
            val resumed = join("resumed-listener")
            assertThat(awaitListenerEvent(resumed) { it is FfiListenerTransportEvent.StreamStarted })
                .isEqualTo(
                    FfiListenerTransportEvent.StreamStarted(
                        "active-stream", resumedAnchor.toULong(), 48_000u, 2u, 240u,
                        "second", "Second track",
                    ),
                )
            assertThat(awaitListenerEvent(resumed) { it is FfiListenerTransportEvent.TrackBoundary })
                .isEqualTo(
                    FfiListenerTransportEvent.TrackBoundary(
                        "active-stream", 2_400_000uL, "third", "Third track",
                    ),
                )
            // A real UDP packet proves that the transport's paused gate
            // was lifted by the authoritative re-anchored StreamStart.
            host.broadcastAudio(
                "active-stream", 0, 48_000, 2, 240, 0, resumedAnchor,
                ByteArray(240 * 2 * 2) { 0x2a.toByte() },
            )
            val audio = awaitListenerEvent(resumed) { it is FfiListenerTransportEvent.AudioReceived }
                as FfiListenerTransportEvent.AudioReceived
            assertThat(audio.sequence).isEqualTo(0uL)
            assertThat(audio.firstSampleIndex).isEqualTo(0uL)
            assertThat(audio.hostPresentationTimeMs).isEqualTo(resumedAnchor.toULong())
            assertThat(audio.payload.toList())
                .containsExactlyElementsIn(ByteArray(240 * 2 * 2) { 0x2a.toByte() }.toList())
            val returningAudio = awaitListenerEvent(returning) {
                it is FfiListenerTransportEvent.AudioReceived
            } as FfiListenerTransportEvent.AudioReceived
            assertThat(returningAudio).isEqualTo(audio)

            // Stop retires the paused/resumed generation even though its
            // presentation anchor is in the future. A new Play deliberately
            // reuses the stream ID, but must not replay the retired track,
            // upcoming boundary or Pause to a newly authorized socket.
            host.broadcastStop("active-stream", SystemClock.elapsedRealtime())
            assertThat(awaitListenerEvent(returning) { it is FfiListenerTransportEvent.Stopped })
                .isInstanceOf(FfiListenerTransportEvent.Stopped::class.java)
            val replayAnchor = resumedAnchor + 60_000L
            host.broadcastStreamStart(
                "active-stream", replayAnchor, 48_000, 2, 240,
                "replay-first", "Replay first track",
            )
            host.broadcastTrackBoundary(
                "active-stream", 240, "replay-second", "Replay second track",
            )
            val replay = join("replay-listener")
            assertThat(awaitListenerEvent(returning) { it is FfiListenerTransportEvent.StreamStarted })
                .isEqualTo(
                    FfiListenerTransportEvent.StreamStarted(
                        "active-stream", replayAnchor.toULong(), 48_000u, 2u, 240u,
                        "replay-first", "Replay first track",
                    ),
                )
            assertThat(awaitListenerEvent(returning) { it is FfiListenerTransportEvent.TrackBoundary })
                .isEqualTo(
                    FfiListenerTransportEvent.TrackBoundary(
                        "active-stream", 240uL, "replay-second", "Replay second track",
                    ),
                )
            assertThat(awaitListenerEvent(replay) { it is FfiListenerTransportEvent.StreamStarted })
                .isEqualTo(
                    FfiListenerTransportEvent.StreamStarted(
                        "active-stream", replayAnchor.toULong(), 48_000u, 2u, 240u,
                        "replay-first", "Replay first track",
                    ),
                )
            assertThat(awaitListenerEvent(replay) { it is FfiListenerTransportEvent.TrackBoundary })
                .isEqualTo(
                    FfiListenerTransportEvent.TrackBoundary(
                        "active-stream", 240uL, "replay-second", "Replay second track",
                    ),
                )

            // A future-dated UDP packet from the retired producer must not
            // preempt packet zero of the new generation with the same ID.
            host.broadcastAudio(
                "active-stream", 101, 48_000, 2, 240, 24_000, replayAnchor + 5_000L,
                ByteArray(240 * 2 * 2) { 0x55.toByte() },
            )
            host.broadcastAudio(
                "active-stream", 0, 48_000, 2, 240, 0, replayAnchor,
                ByteArray(240 * 2 * 2) { 0x35.toByte() },
            )
            val replayAudio = awaitListenerEvent(replay) {
                it is FfiListenerTransportEvent.AudioReceived
            } as FfiListenerTransportEvent.AudioReceived
            assertThat(replayAudio.sequence).isEqualTo(0uL)
            assertThat(replayAudio.firstSampleIndex).isEqualTo(0uL)
            assertThat(replayAudio.hostPresentationTimeMs).isEqualTo(replayAnchor.toULong())
            assertThat(replayAudio.payload.toList())
                .containsExactlyElementsIn(ByteArray(240 * 2 * 2) { 0x35.toByte() }.toList())
            val returningReplayAudio = awaitListenerEvent(returning) {
                it is FfiListenerTransportEvent.AudioReceived
            } as FfiListenerTransportEvent.AudioReceived
            assertThat(returningReplayAudio).isEqualTo(replayAudio)
        } finally {
            syncKeepAlives.forEach { it.cancelAndJoin() }
            listeners.forEach { runCatching { it.shutdown() } }
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
