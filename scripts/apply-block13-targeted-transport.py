#!/usr/bin/env python3
"""Apply the exact-listener Android control transport migration."""

from __future__ import annotations

from pathlib import Path
from textwrap import dedent

ROOT = Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def write(path: str, content: str) -> None:
    (ROOT / path).write_text(content, encoding="utf-8")


def replace_once(path: str, label: str, old: str, new: str) -> None:
    content = read(path)
    count = content.count(old)
    if count != 1:
        raise SystemExit(f"{path} [{label}]: expected one match, found {count}")
    write(path, content.replace(old, new))


def replace_range(path: str, label: str, start: str, end: str, replacement: str) -> None:
    content = read(path)
    if content.count(start) != 1 or content.count(end) != 1:
        raise SystemExit(
            f"{path} [{label}]: start={content.count(start)} end={content.count(end)}"
        )
    start_index = content.index(start)
    end_index = content.index(end, start_index)
    write(path, content[:start_index] + replacement + content[end_index:])


def update_transport_models() -> None:
    path = "app/src/main/java/com/ekkus/silentdisco/core/transport/TransportModels.kt"
    targeted_result = dedent(
        """
        data class TargetedDeliveryResult(
            val listenerId: String,
            val intendedPeerCount: Int,
            val successCount: Int,
            val failureCount: Int,
            val message: String? = null,
        ) {
            val deliveredToTarget: Boolean
                get() = intendedPeerCount == 1 && successCount == 1 && failureCount == 0

            companion object {
                fun delivered(listenerId: String) = TargetedDeliveryResult(
                    listenerId = listenerId,
                    intendedPeerCount = 1,
                    successCount = 1,
                    failureCount = 0,
                )

                fun notFound(listenerId: String, message: String) = TargetedDeliveryResult(
                    listenerId = listenerId,
                    intendedPeerCount = 0,
                    successCount = 0,
                    failureCount = 0,
                    message = message,
                )

                fun failed(listenerId: String, message: String) = TargetedDeliveryResult(
                    listenerId = listenerId,
                    intendedPeerCount = 1,
                    successCount = 0,
                    failureCount = 1,
                    message = message,
                )
            }
        }

        """
    )
    replace_once(
        path,
        "targeted-result",
        "enum class BroadcastDeliverySeverity {",
        targeted_result + "enum class BroadcastDeliverySeverity {",
    )
    replace_once(
        path,
        "transport-interface",
        """    suspend fun sendControlToHost(message: ControlMessage)\n    suspend fun broadcastControl(message: ControlMessage): SendAllResult\n""",
        """    suspend fun sendControlToHost(message: ControlMessage)\n    suspend fun sendControlToListener(\n        listenerId: String,\n        message: ControlMessage,\n    ): TargetedDeliveryResult\n    suspend fun broadcastControl(message: ControlMessage): SendAllResult\n""",
    )


def update_tcp_transport() -> None:
    path = "app/src/main/java/com/ekkus/silentdisco/core/transport/TcpTransport.kt"
    peer_result = dedent(
        """
        internal data class PeerDeliveryResult(
            val peerFound: Boolean,
            val delivered: Boolean,
            val errorMessage: String? = null,
        )

        """
    )
    replace_once(
        path,
        "peer-result",
        "internal class TcpServerChannel<T>(",
        peer_result + "internal class TcpServerChannel<T>(",
    )
    targeted_send = dedent(
        """
            suspend fun sendTo(remoteAddress: String, message: T): PeerDeliveryResult {
                val peer = peers[remoteAddress]
                    ?: return PeerDeliveryResult(
                        peerFound = false,
                        delivered = false,
                        errorMessage = "No active $channelName connection for $remoteAddress",
                    )
                return runCatching {
                    peer.send(message)
                    PeerDeliveryResult(peerFound = true, delivered = true)
                }.getOrElse { error ->
                    val messageText = error.message ?: "Unknown $channelName send failure"
                    logger.w(
                        "transport.$channelName.send-target",
                        "Failed to send to $remoteAddress: $messageText",
                    )
                    peer.close()
                    PeerDeliveryResult(
                        peerFound = true,
                        delivered = false,
                        errorMessage = messageText,
                    )
                }
            }

        """
    )
    replace_once(
        path,
        "targeted-send",
        "    fun connectionCount(): Int = peers.size",
        targeted_send + "    fun connectionCount(): Int = peers.size",
    )


def update_wifi_direct_transport() -> None:
    path = "app/src/main/java/com/ekkus/silentdisco/core/transport/WifiDirectTransportService.kt"
    replace_once(
        path,
        "route-map-import",
        "import kotlinx.coroutines.CoroutineScope",
        "import java.util.concurrent.ConcurrentHashMap\nimport kotlinx.coroutines.CoroutineScope",
    )
    replace_once(
        path,
        "route-map-field",
        "    private val currentPeers = linkedMapOf<String, WifiP2pDevice>()",
        """    private val currentPeers = linkedMapOf<String, WifiP2pDevice>()\n    private val listenerControlRoutes = ConcurrentHashMap<String, String>()""",
    )
    replace_once(
        path,
        "clear-routes-on-host-start",
        "        hosting = true\n",
        "        hosting = true\n        listenerControlRoutes.clear()\n",
    )

    targeted_method = dedent(
        """
            override suspend fun sendControlToListener(
                listenerId: String,
                message: ControlMessage,
            ): TargetedDeliveryResult {
                val embeddedListenerId = message.targetListenerId()
                if (embeddedListenerId != listenerId) {
                    return TargetedDeliveryResult.failed(
                        listenerId,
                        "Control message target does not match requested listener",
                    )
                }
                val remoteAddress = listenerControlRoutes[listenerId]
                    ?: return TargetedDeliveryResult.notFound(
                        listenerId,
                        "No control connection is associated with listener $listenerId",
                    )
                val server = controlServer
                    ?: return TargetedDeliveryResult.failed(listenerId, "Control server is not active")
                val delivery = server.sendTo(remoteAddress, message)
                if (!delivery.peerFound || !delivery.delivered) {
                    listenerControlRoutes.remove(listenerId, remoteAddress)
                }
                recordHeartbeat()
                updateByteCounts()
                return when {
                    delivery.delivered -> TargetedDeliveryResult.delivered(listenerId)
                    delivery.peerFound -> TargetedDeliveryResult.failed(
                        listenerId,
                        delivery.errorMessage ?: "Targeted control delivery failed",
                    )
                    else -> TargetedDeliveryResult.notFound(
                        listenerId,
                        delivery.errorMessage ?: "Targeted listener connection is no longer active",
                    )
                }
            }

        """
    )
    replace_once(
        path,
        "targeted-control-method",
        "    override suspend fun broadcastControl(message: ControlMessage): SendAllResult {",
        targeted_method + "    override suspend fun broadcastControl(message: ControlMessage): SendAllResult {",
    )

    observe_control = dedent(
        """
            private fun observeControlServer(server: TcpServerChannel<ControlMessage>) {
                scope.launch {
                    server.incoming.collect { event ->
                        val message = event.message
                        if (message is ControlMessage.JoinRequest) {
                            listenerControlRoutes[message.device.deviceId] = event.remoteAddress
                        }
                        _controlMessages.emit(message)
                        recordHeartbeat()
                        updateByteCounts()
                        logger.d("transport.message", "Received message from ${event.remoteAddress}")
                    }
                }
            }

        """
    )
    replace_range(
        path,
        "control-route-observer",
        "    private fun observeControlServer(server: TcpServerChannel<ControlMessage>) {",
        "    private fun observeSyncServer(server: TcpServerChannel<SyncRequestPacket>) {",
        observe_control,
    )

    content = read(path)
    try_emit_count = content.count("::tryEmit")
    if try_emit_count < 4:
        raise SystemExit(f"{path} [suspending-forwarding]: expected >=4 tryEmit uses, found {try_emit_count}")
    write(path, content.replace("::tryEmit", "::emit"))
    replace_once(
        path,
        "suspending-collector",
        "private fun <T> SharedFlow<TransportEvent<T>>.collectInto(emit: (T) -> Boolean)",
        "private fun <T> SharedFlow<TransportEvent<T>>.collectInto(emit: suspend (T) -> Unit)",
    )
    replace_once(
        path,
        "clear-routes-on-stop",
        "    private fun stopServerChannels() {\n",
        "    private fun stopServerChannels() {\n        listenerControlRoutes.clear()\n",
    )

    target_helper = dedent(
        """
            private fun ControlMessage.targetListenerId(): String? = when (this) {
                is ControlMessage.JoinApproval -> listenerId
                is ControlMessage.JoinRejection -> listenerId
                is ControlMessage.Heartbeat -> listenerId
                is ControlMessage.Disconnect -> listenerId
                is ControlMessage.ResyncNotice -> listenerId
                is ControlMessage.Hello,
                is ControlMessage.JoinRequest,
                is ControlMessage.StreamStart,
                is ControlMessage.Pause,
                is ControlMessage.Stop,
                -> null
            }

        """
    )
    replace_once(
        path,
        "target-validation-helper",
        "    private companion object {",
        target_helper + "    private companion object {",
    )


def write_targeted_transport_test() -> None:
    path = ROOT / "app/src/test/java/com/ekkus/silentdisco/core/transport/TargetedTcpTransportTest.kt"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        dedent(
            """
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
            """
        ).lstrip(),
        encoding="utf-8",
    )


def main() -> None:
    update_transport_models()
    update_tcp_transport()
    update_wifi_direct_transport()
    write_targeted_transport_test()


if __name__ == "__main__":
    main()
