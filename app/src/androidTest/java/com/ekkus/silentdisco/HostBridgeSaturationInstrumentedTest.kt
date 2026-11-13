package com.ekkus.silentdisco

import android.os.SystemClock
import com.ekkus.silentdisco.core.rust.HostTransportController
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportException
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportHandle
import com.google.common.truth.Truth.assertThat
import java.net.ServerSocket
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.withTimeoutOrNull
import org.junit.Test

/**
 * Cross-layer saturation: a stalled Android collector suspends the native poll,
 * which backs the Rust host event queue up behind it. The host must then refuse
 * new joins visibly and in bounded time, never admit a refused peer, keep
 * delivering Stop to a live listener, and, once collection resumes, deliver
 * every join it accepted in order with no gaps.
 */
class HostBridgeSaturationInstrumentedTest {
    private val v4 = V4TestListeners()

    @Test
    fun stalledCollectorRefusesJoinsVisiblyKeepsStopFlowingAndLosesNoAcceptedJoin() = runBlocking<Unit> {
        val host = HostTransportController()
        val handles = mutableListOf<FfiListenerTransportHandle>()
        val keepAlives = mutableListOf<Job>()
        try {
            val reserved = List(3) { ServerSocket(0) }
            val ports = reserved.map(ServerSocket::getLocalPort)
            reserved.forEach(ServerSocket::close)
            val sessionId = "saturation-${SystemClock.elapsedRealtimeNanos()}"
            host.bind(this, "127.0.0.1", ports[0], ports[1], ports[2], sessionId)
            val endpoint = """
                {"hostAddress":"127.0.0.1","controlPort":${ports[0]},"syncPort":${ports[1]},
                "audioPort":${ports[2]},"sessionId":"$sessionId","protocolVersion":4,
                "inviteCodeRequired":false,"expiresAtMs":null}
            """.trimIndent().replace("\n", "")

            // A live, approved listener with a running stream, while the host
            // event collector is still active.
            val live = connect(endpoint, "live-listener")
            handles += live
            io { live.sendJoinRequest("live-listener", null) }
            val liveJoin = withTimeout(BOUND_MS) {
                host.events.first { it is FfiHostTransportEvent.JoinRequestReceived }
            } as FfiHostTransportEvent.JoinRequestReceived
            assertThat(host.sendJoinApproval(liveJoin.deviceId, false)?.successfulPeers).isEqualTo(1u)
            host.authorizeListener(liveJoin.deviceId, liveJoin.syncPort, liveJoin.audioPort)
            awaitListener(live) { it is FfiListenerTransportEvent.JoinApproved }
            keepAlives += launch(Dispatchers.IO) {
                var correlation = 1uL
                while (isActive) {
                    live.sendSyncRequest(correlation++, (System.nanoTime() / 1_000_000L).toULong())
                    delay(2_000)
                }
            }
            host.broadcastStreamStart(
                "saturation-stream", SystemClock.elapsedRealtime() + 600_000L, 48_000, 2, 240,
                "entry", "Saturation song",
            )
            awaitListener(live) { it is FfiListenerTransportEvent.StreamStarted }

            // From here nothing collects host events: the Android queue fills,
            // the native poll suspends and the Rust queue backs up.
            var refusedAt: Int? = null
            for (cycle in 0 until MAX_CYCLES) {
                if (attemptJoin(endpoint, "sat-$cycle") != Attempt.ACCEPTED) {
                    refusedAt = cycle
                    break
                }
            }
            assertThat(refusedAt).isNotNull()
            val accepted = refusedAt!!
            android.util.Log.i("HostBridgeSaturation", "first refusal after $accepted accepted joins")
            // Android alone absorbs about 21 cycles (64 slots, 3 events each);
            // more than that proves the Rust queue backed up behind the stall.
            assertThat(accepted).isAtLeast(MIN_ABSORBED_JOINS)

            // A further joiner is refused again (visibly, within bounds) and
            // never admitted.
            assertThat(attemptJoin(endpoint, "late-joiner")).isNotEqualTo(Attempt.ACCEPTED)

            // Stop still reaches the live listener while saturated.
            val stopped = host.broadcastStop("saturation-stream", SystemClock.elapsedRealtime())
            assertThat(stopped?.successfulPeers).isEqualTo(1u)
            awaitListener(live) { it is FfiListenerTransportEvent.Stopped }

            // Resume collection: every accepted join arrives, in order, no gaps,
            // and no refused cycle appears as an accepted join.
            // v4 device IDs are key-bound, so map each back to its cycle; the
            // refused cycle is mapped too, so a leaked refusal would show up.
            val cycleById = (0..accepted).associateBy { v4.id("sat-$it") }
            val joined = mutableListOf<Int>()
            withTimeoutOrNull(DRAIN_MS) {
                host.events.collect { event ->
                    if (event is FfiHostTransportEvent.JoinRequestReceived) {
                        cycleById[event.deviceId]?.let { joined += it }
                    }
                }
            }
            assertThat(joined).isEqualTo((0 until accepted).toList())
        } finally {
            keepAlives.forEach { it.cancel() }
            withTimeout(BOUND_MS) {
                host.close()
                host.close()
            }
            handles.forEach {
                runCatching { it.shutdown() }
                runCatching { it.close() }
            }
            v4.close()
        }
    }

    private enum class Attempt { ACCEPTED, REFUSED }

    private suspend fun connect(endpoint: String, device: String): FfiListenerTransportHandle =
        withTimeout(BOUND_MS) {
            io {
                v4.connect(endpoint, device)
            }
        }

    /** One connect, join and disconnect; refusal can be a failed connect, a rejection or a close. */
    private suspend fun attemptJoin(endpoint: String, device: String): Attempt {
        val handle = try {
            connect(endpoint, device)
        } catch (_: FfiListenerTransportException) {
            return Attempt.REFUSED
        } catch (_: TimeoutCancellationException) {
            return Attempt.REFUSED
        }
        try {
            io { handle.sendJoinRequest(device, null) }
            val deadline = SystemClock.elapsedRealtime() + ATTEMPT_WINDOW_MS
            while (SystemClock.elapsedRealtime() < deadline) {
                val event = try {
                    io { handle.pollEvent(50uL) }
                } catch (_: FfiListenerTransportException) {
                    return Attempt.REFUSED
                }
                when (event) {
                    is FfiListenerTransportEvent.JoinRejected,
                    is FfiListenerTransportEvent.ConnectionClosed -> return Attempt.REFUSED
                    is FfiListenerTransportEvent.JoinApproved ->
                        error("unapproved joiner $device was admitted")
                    else -> Unit
                }
            }
            return Attempt.ACCEPTED
        } finally {
            runCatching { io { handle.shutdown() } }
            runCatching { io { handle.close() } }
        }
    }

    private suspend fun awaitListener(
        listener: FfiListenerTransportHandle,
        predicate: (FfiListenerTransportEvent) -> Boolean,
    ): FfiListenerTransportEvent = withTimeout(BOUND_MS) {
        io {
            var matched: FfiListenerTransportEvent? = null
            while (matched == null) {
                ensureActive()
                matched = listener.pollEvent(500uL)?.takeIf(predicate)
            }
            matched
        }
    }

    private suspend fun <T> io(block: () -> T): T = withContext(Dispatchers.IO) { block() }

    private companion object {
        const val BOUND_MS = 30_000L
        const val DRAIN_MS = 15_000L
        const val ATTEMPT_WINDOW_MS = 150L
        const val MAX_CYCLES = 400
        const val MIN_ABSORBED_JOINS = 30
    }
}
