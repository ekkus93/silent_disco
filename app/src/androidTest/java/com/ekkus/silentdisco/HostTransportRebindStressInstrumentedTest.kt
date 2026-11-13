package com.ekkus.silentdisco

import android.os.SystemClock
import com.ekkus.silentdisco.core.rust.HostTransportController
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportException
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportHandle
import com.google.common.truth.Truth.assertThat
import java.net.ServerSocket
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.Test

/**
 * Real native TCP/UDP host rebind under repetition. Every cycle binds a fresh
 * host session on new ports while the previous session's listener socket is
 * still open. A cycle may only see its own join, may not be approved or
 * authorized through the previous session's peer, and must complete within a
 * bounded time, so a poll that never exits or a handle that is not retired
 * would stall or fail the run.
 */
class HostTransportRebindStressInstrumentedTest {
    @Test
    fun repeatedRebindNeverCrossesSessionsAndAlwaysCompletesWithinBounds() = runBlocking<Unit> {
        val host = HostTransportController()
        val listeners = mutableListOf<FfiListenerTransportHandle>()
        val v4 = V4TestListeners()
        try {
            repeat(CYCLES) { cycle ->
                val device = "stress-device-$cycle"
                // The endpoint and the host must name the same session.
                val sessionId = "stress-session-$cycle-${SystemClock.elapsedRealtimeNanos()}"
                val reserved = List(3) { ServerSocket(0) }
                val ports = reserved.map(ServerSocket::getLocalPort)
                reserved.forEach(ServerSocket::close)
                // The previous session is retired here while its listener
                // socket remains open on the client side.
                step(cycle, "host.bind") {
                    host.bind(
                        scope = this@runBlocking,
                        localAddress = "127.0.0.1",
                        controlPort = ports[0],
                        syncPort = ports[1],
                        audioPort = ports[2],
                        sessionId = sessionId,
                    )
                }
                val endpoint = """
                    {"hostAddress":"127.0.0.1","controlPort":${ports[0]},"syncPort":${ports[1]},
                    "audioPort":${ports[2]},"sessionId":"$sessionId","protocolVersion":4,
                    "inviteCodeRequired":false,"expiresAtMs":null}
                """.trimIndent().replace("\n", "")

                val listener = step(cycle, "listener.connect") {
                    withContext(Dispatchers.IO) {
                        v4.connect(endpoint, device)
                    }
                }
                listeners += listener
                withContext(Dispatchers.IO) { listener.sendJoinRequest(device, null) }

                // Only this session's join may arrive: a queued join from a
                // retired host would carry an earlier device ID.
                val join = step(cycle, "host JoinRequestReceived") {
                    host.events.first { it is FfiHostTransportEvent.JoinRequestReceived }
                } as FfiHostTransportEvent.JoinRequestReceived
                assertThat(join.deviceId).isEqualTo(v4.id(device))

                if (cycle > 0) {
                    // The previous session's peer does not exist on this host.
                    val stale = try {
                        host.sendJoinApproval(v4.id("stress-device-${cycle - 1}"), false)?.successfulPeers ?: 0u
                    } catch (_: FfiHostTransportException) {
                        0u
                    }
                    assertThat(stale).isEqualTo(0u)
                }

                assertThat(host.sendJoinApproval(v4.id(device), false)?.successfulPeers).isEqualTo(1u)
                host.authorizeListener(join.deviceId, join.syncPort, join.audioPort)
                step(cycle, "listener JoinApproved") {
                    awaitListenerEvent(listener) { it is FfiListenerTransportEvent.JoinApproved }
                }

                // Leave a join in the host's event buffer that nothing consumes.
                // The next rebind must discard it: if it leaked, the next
                // cycle's first join would be this ghost, not its own device.
                val ghost = withContext(Dispatchers.IO) {
                    v4.connect(endpoint, "ghost-$cycle")
                        .also { it.sendJoinRequest("ghost-$cycle", null) }
                }
                listeners += ghost
                delay(150)
            }
        } finally {
            // Closing must also complete in bounded time and be repeatable.
            withTimeout(BOUND_MS) {
                host.close()
                host.close()
            }
            listeners.forEach {
                runCatching { it.shutdown() }
                runCatching { it.close() }
            }
            v4.close()
        }
    }

    /** A bare timeout does not say which cycle or step stalled. */
    private suspend fun <T> step(cycle: Int, name: String, block: suspend () -> T): T = try {
        withTimeout(BOUND_MS) { block() }
    } catch (error: TimeoutCancellationException) {
        throw AssertionError("cycle $cycle timed out at step '$name' after $BOUND_MS ms", error)
    }

    private suspend fun awaitListenerEvent(
        listener: FfiListenerTransportHandle,
        predicate: (FfiListenerTransportEvent) -> Boolean,
    ): FfiListenerTransportEvent = withTimeout(BOUND_MS) {
        withContext(Dispatchers.IO) {
            var matched: FfiListenerTransportEvent? = null
            while (matched == null) {
                // The native poll is bounded to 500 ms; checking for
                // cancellation between polls lets the timeout end the wait.
                ensureActive()
                matched = listener.pollEvent(500uL)?.takeIf(predicate)
            }
            matched
        }
    }

    private companion object {
        const val CYCLES = 25
        const val BOUND_MS = 15_000L
    }
}
