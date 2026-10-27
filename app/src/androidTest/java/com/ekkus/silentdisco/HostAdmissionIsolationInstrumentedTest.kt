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

/**
 * Real TCP/UDP admission regression: a rejected listener must never receive
 * cached playback state or audio, while a subsequent approved listener joins
 * the same running host at the authoritative track and sample timeline.
 */
class HostAdmissionIsolationInstrumentedTest {
    @Test
    fun rejectedJoinCannotHearStreamAndLaterApprovalReplaysCurrentQueue() = runBlocking<Unit> {
        val sockets = List(3) { ServerSocket(0) }
        val ports = sockets.map(ServerSocket::getLocalPort)
        sockets.forEach(ServerSocket::close)
        val sessionId = "android-admission-${System.nanoTime()}"
        val host = HostTransportController()
        val listeners = mutableListOf<FfiListenerTransportHandle>()
        var heartbeat: Job? = null
        try {
            host.bind(this, "127.0.0.1", ports[0], ports[1], ports[2], sessionId)
            val startMs = SystemClock.elapsedRealtime() + 600_000L
            host.broadcastStreamStart(
                "admission-stream", startMs, 48_000, 2, 240, "first", "First song",
            )
            host.broadcastTrackBoundary(
                "admission-stream", 24_000, "second", "Second song",
            )
            val endpoint = """
                {"hostAddress":"127.0.0.1","controlPort":${ports[0]},"syncPort":${ports[1]},
                "audioPort":${ports[2]},"sessionId":"$sessionId","protocolVersion":3,
                "inviteCodeRequired":false,"expiresAtMs":null}
            """.trimIndent().replace("\n", "")

            val denied = FfiListenerTransportHandle.connect(
                endpoint, System.currentTimeMillis().toULong(), "denied-device", "127.0.0.1",
            )
            listeners += denied
            denied.sendJoinRequest("Denied Listener", null)
            val deniedRequest = withTimeout(10_000) {
                host.events.first {
                    it is FfiHostTransportEvent.JoinRequestReceived &&
                        it.deviceId == "denied-device"
                }
            } as FfiHostTransportEvent.JoinRequestReceived
            val rejection = host.sendJoinRejection(deniedRequest.deviceId, "Host declined")
            assertThat(rejection?.successfulPeers).isEqualTo(1u)
            val deniedEvent = awaitAdmissionEvent(denied) {
                it is FfiListenerTransportEvent.JoinRejected
            } as FfiListenerTransportEvent.JoinRejected
            assertThat(deniedEvent.reason).isEqualTo("Host declined")

            // Rejecting a TCP join must not accidentally authorize its UDP
            // route, even though the host has an active cached stream.
            val noRoute = host.broadcastAudio(
                "admission-stream", 100, 48_000, 2, 240, 24_000, startMs + 500L,
                ByteArray(240 * 2 * 2) { 0x21.toByte() },
            )
            assertThat(noRoute?.intendedPeers).isEqualTo(0u)

            val approved = FfiListenerTransportHandle.connect(
                endpoint, System.currentTimeMillis().toULong(), "approved-device", "127.0.0.1",
            )
            listeners += approved
            approved.sendJoinRequest("Approved Listener", null)
            val approvedRequest = withTimeout(10_000) {
                host.events.first {
                    it is FfiHostTransportEvent.JoinRequestReceived &&
                        it.deviceId == "approved-device"
                }
            } as FfiHostTransportEvent.JoinRequestReceived
            host.sendJoinApproval(approvedRequest.deviceId, false)
            assertThat(awaitAdmissionEvent(approved) {
                it is FfiListenerTransportEvent.JoinApproved
            }).isInstanceOf(FfiListenerTransportEvent.JoinApproved::class.java)
            host.authorizeListener(
                approvedRequest.deviceId, approvedRequest.syncPort, approvedRequest.audioPort,
            )
            // Production host liveness expires peers without sync requests;
            // exercise the real heartbeat during slow emulator assertions.
            heartbeat = launch(Dispatchers.IO) {
                var correlationId = 1uL
                while (isActive) {
                    approved.sendSyncRequest(
                        correlationId++, SystemClock.elapsedRealtime().toULong(),
                    )
                    delay(2_000L)
                }
            }

            assertThat(awaitAdmissionEvent(approved) {
                it is FfiListenerTransportEvent.StreamStarted
            }).isEqualTo(
                FfiListenerTransportEvent.StreamStarted(
                    "admission-stream", startMs.toULong(), 48_000u, 2u, 240u,
                    "first", "First song",
                ),
            )
            assertThat(awaitAdmissionEvent(approved) {
                it is FfiListenerTransportEvent.TrackBoundary
            }).isEqualTo(
                FfiListenerTransportEvent.TrackBoundary(
                    "admission-stream", 24_000uL, "second", "Second song",
                ),
            )

            val pcm = ByteArray(240 * 2 * 2) { 0x35.toByte() }
            val delivered = host.broadcastAudio(
                "admission-stream", 101, 48_000, 2, 240, 24_240, startMs + 505L, pcm,
            )
            assertThat(delivered?.successfulPeers).isEqualTo(1u)
            val audio = awaitAdmissionEvent(approved) {
                it is FfiListenerTransportEvent.AudioReceived
            } as FfiListenerTransportEvent.AudioReceived
            assertThat(audio.sequence).isEqualTo(101uL)
            assertThat(audio.firstSampleIndex).isEqualTo(24_240uL)
            assertThat(audio.hostPresentationTimeMs).isEqualTo((startMs + 505L).toULong())
            assertThat(audio.payload.toList()).containsExactlyElementsIn(pcm.toList()).inOrder()

            // Revoking the approved listener must retire its audio route too.
            heartbeat?.cancelAndJoin()
            heartbeat = null
            assertThat(host.disconnectPeer("approved-device", "Session access revoked")?.successfulPeers)
                .isEqualTo(1u)
            assertThat(awaitAdmissionEvent(approved) {
                it is FfiListenerTransportEvent.HostDisconnected
            }).isEqualTo(FfiListenerTransportEvent.HostDisconnected("Session access revoked"))
            assertThat(host.broadcastAudio(
                "admission-stream", 102, 48_000, 2, 240, 24_480, startMs + 510L, pcm,
            )?.intendedPeers).isEqualTo(0u)
        } finally {
            heartbeat?.cancelAndJoin()
            listeners.forEach { runCatching { it.shutdown() } }
            host.close()
        }
    }
}

private suspend fun awaitAdmissionEvent(
    listener: FfiListenerTransportHandle,
    predicate: (FfiListenerTransportEvent) -> Boolean,
): FfiListenerTransportEvent = withTimeout(10_000) {
    while (true) {
        val event = listener.pollEvent(500uL)
        if (event != null && predicate(event)) return@withTimeout event
    }
    error("Timed out waiting for listener event")
}
