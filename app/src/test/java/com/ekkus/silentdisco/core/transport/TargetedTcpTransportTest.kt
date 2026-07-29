package com.ekkus.silentdisco.core.transport

import com.ekkus.silentdisco.core.logging.AppLogger
import com.ekkus.silentdisco.core.protocol.ControlMessage
import com.ekkus.silentdisco.core.protocol.DeviceIdentity
import com.ekkus.silentdisco.core.protocol.SessionId
import com.google.common.truth.Truth.assertThat
import java.net.ServerSocket
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Test

class TargetedTcpTransportTest {
    @Test
    fun `server sends approval only to source connection`() = runBlocking {
        val port = ServerSocket(0).use { it.localPort }
        val logger = AppLogger()
        val server = TcpServerChannel(
            port = port,
            channelName = "control-test",
            codec = ControlMessageCodec,
            logger = logger,
        )
        val intendedClient = TcpClientChannel(
            host = "127.0.0.1",
            port = port,
            channelName = "intended-client",
            codec = ControlMessageCodec,
            logger = logger,
        )
        val unrelatedClient = TcpClientChannel(
            host = "127.0.0.1",
            port = port,
            channelName = "unrelated-client",
            codec = ControlMessageCodec,
            logger = logger,
        )

        try {
            server.start()
            intendedClient.connect()
            unrelatedClient.connect()

            val inbound = async(start = CoroutineStart.UNDISPATCHED) {
                withTimeout(5_000) { server.incoming.first() }
            }
            intendedClient.send(
                ControlMessage.JoinRequest(
                    version = 1,
                    sessionId = SessionId("session-1"),
                    device = DeviceIdentity("listener-1", "Pixel"),
                ),
            )
            val sourceEvent = inbound.await()

            val intendedReply = async(start = CoroutineStart.UNDISPATCHED) {
                withTimeout(5_000) { intendedClient.incoming.first() }
            }
            val approval = ControlMessage.JoinApproval(
                version = 1,
                sessionId = SessionId("session-1"),
                listenerId = "listener-1",
                trustedForFuture = false,
            )
            val result = server.sendTo(sourceEvent.remoteAddress, approval)

            assertThat(result.peerFound).isTrue()
            assertThat(result.delivered).isTrue()
            assertThat(intendedReply.await().message).isEqualTo(approval)
            assertThat(unrelatedClient.bytesReceived()).isEqualTo(0L)
        } finally {
            unrelatedClient.close()
            intendedClient.close()
            server.close()
        }
    }

    @Test
    fun `server reports missing targeted connection`() = runBlocking {
        val port = ServerSocket(0).use { it.localPort }
        val server = TcpServerChannel(
            port = port,
            channelName = "control-test",
            codec = ControlMessageCodec,
            logger = AppLogger(),
        )
        try {
            server.start()
            val result = server.sendTo(
                "/127.0.0.1:1",
                ControlMessage.JoinRejection(
                    version = 1,
                    sessionId = SessionId("session-1"),
                    listenerId = "listener-1",
                    reason = "rejected",
                ),
            )
            assertThat(result.peerFound).isFalse()
            assertThat(result.delivered).isFalse()
        } finally {
            server.close()
        }
    }
}
