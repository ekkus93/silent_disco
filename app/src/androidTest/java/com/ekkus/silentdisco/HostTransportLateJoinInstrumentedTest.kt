package com.ekkus.silentdisco

import com.ekkus.silentdisco.core.rust.HostTransportController
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportHandle
import com.google.common.truth.Truth.assertThat
import java.net.ServerSocket
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.flow.first
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
                hostStartTimeMs = 1_000,
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

            assertThat(awaitListenerEvent(listener) { it is FfiListenerTransportEvent.StreamStarted }).isEqualTo(
                FfiListenerTransportEvent.StreamStarted(
                    streamId = "active-stream",
                    hostStartTimeMs = 1_000uL,
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
                hostPresentationTimeMs = 1_500,
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
            assertThat(audio.hostPresentationTimeMs).isEqualTo(1_500uL)
            assertThat(audio.payload.toList()).containsExactlyElementsIn(ByteArray(240 * 2 * 2).toList())
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
