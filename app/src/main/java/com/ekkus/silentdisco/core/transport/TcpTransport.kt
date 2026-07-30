package com.ekkus.silentdisco.core.transport

import com.ekkus.silentdisco.core.logging.AppLogger
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.ControlMessage
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.protocol.SyncRequestPacket
import com.ekkus.silentdisco.core.protocol.SyncResponsePacket
import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.io.Closeable
import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.EOFException
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket
import java.net.SocketException
import java.nio.ByteBuffer
import java.nio.charset.StandardCharsets
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.KSerializer
import kotlinx.serialization.json.Json

data class TransportPorts(
    val control: Int = 41_000,
    val sync: Int = 41_001,
    val audio: Int = 41_002,
)

internal interface MessageCodec<T> {
    fun encode(value: T): ByteArray
    fun decode(bytes: ByteArray): T
}

internal class JsonMessageCodec<T>(
    private val serializer: KSerializer<T>,
) : MessageCodec<T> {
    private val json = Json {
        encodeDefaults = true
        ignoreUnknownKeys = true
        classDiscriminator = "type"
    }

    override fun encode(value: T): ByteArray =
        json.encodeToString(serializer, value).encodeToByteArray()

    override fun decode(bytes: ByteArray): T =
        json.decodeFromString(serializer, bytes.toString(StandardCharsets.UTF_8))
}

internal object AudioPacketCodec : MessageCodec<AudioPacket> {
    override fun encode(value: AudioPacket): ByteArray {
        val sessionIdBytes = value.sessionId.value.encodeToByteArray()
        val streamIdBytes = value.streamId.value.encodeToByteArray()
        val codecBytes = value.codec.encodeToByteArray()
        val payload = value.payload
        val buffer = ByteBuffer.allocate(
            Int.SIZE_BYTES * 10 +
                Long.SIZE_BYTES * 3 +
                sessionIdBytes.size +
                streamIdBytes.size +
                codecBytes.size +
                payload.size,
        )
        buffer.putInt(value.version)
        putBytes(buffer, sessionIdBytes)
        putBytes(buffer, streamIdBytes)
        buffer.putLong(value.sequenceNumber)
        putBytes(buffer, codecBytes)
        buffer.putInt(value.sampleRate)
        buffer.putInt(value.channelCount)
        buffer.putInt(value.samplesPerPacket)
        buffer.putLong(value.firstSampleIndex)
        buffer.putLong(value.hostPresentationTimeMs)
        putBytes(buffer, payload)
        buffer.putInt(value.checksum ?: Int.MIN_VALUE)
        return buffer.array()
    }

    override fun decode(bytes: ByteArray): AudioPacket {
        val buffer = ByteBuffer.wrap(bytes)
        return AudioPacket(
            version = buffer.int,
            sessionId = SessionId(readBytes(buffer).toString(StandardCharsets.UTF_8)),
            streamId = StreamId(readBytes(buffer).toString(StandardCharsets.UTF_8)),
            sequenceNumber = buffer.long,
            codec = readBytes(buffer).toString(StandardCharsets.UTF_8),
            sampleRate = buffer.int,
            channelCount = buffer.int,
            samplesPerPacket = buffer.int,
            firstSampleIndex = buffer.long,
            hostPresentationTimeMs = buffer.long,
            payload = readBytes(buffer),
            checksum = buffer.int.takeUnless { it == Int.MIN_VALUE },
        )
    }

    private fun putBytes(buffer: ByteBuffer, bytes: ByteArray) {
        buffer.putInt(bytes.size)
        buffer.put(bytes)
    }

    private fun readBytes(buffer: ByteBuffer): ByteArray {
        val size = buffer.int
        return ByteArray(size).also(buffer::get)
    }
}

internal data class TransportEvent<T>(
    val remoteAddress: String,
    val message: T,
)

internal data class TcpChannelStats(
    val connectionCount: Int = 0,
    val bytesSent: Long = 0,
    val bytesReceived: Long = 0,
)


internal data class PeerDeliveryResult(
    val peerFound: Boolean,
    val delivered: Boolean,
    val errorMessage: String? = null,
)

internal class TcpServerChannel<T>(
    private val port: Int,
    private val channelName: String,
    private val codec: MessageCodec<T>,
    private val logger: AppLogger,
) : Closeable {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val _incoming = MutableSharedFlow<TransportEvent<T>>(extraBufferCapacity = 64)
    val incoming: SharedFlow<TransportEvent<T>> = _incoming.asSharedFlow()
    private val _stats = MutableSharedFlow<TcpChannelStats>(replay = 1, extraBufferCapacity = 4)
    val stats: SharedFlow<TcpChannelStats> = _stats.asSharedFlow()

    private val peers = ConcurrentHashMap<String, PeerConnection<T>>()
    private val bytesSent = AtomicLong(0)
    private val bytesReceived = AtomicLong(0)
    private var acceptJob: Job? = null
    private var serverSocket: ServerSocket? = null

    fun start() {
        if (serverSocket != null) return
        serverSocket = ServerSocket().apply { reuseAddress = true }
        serverSocket?.bind(InetSocketAddress(port))
        emitStats()
        acceptJob = scope.launch {
            while (true) {
                val socket = try {
                    serverSocket?.accept() ?: break
                } catch (error: SocketException) {
                    if (serverSocket?.isClosed == true) break
                    throw error
                }
                val remoteAddress = socket.remoteSocketAddress.toString()
                val peer = PeerConnection(
                    socket = socket,
                    logger = logger,
                    channelName = channelName,
                    remoteAddress = remoteAddress,
                    codec = codec,
                    onBytesSent = { delta ->
                        bytesSent.addAndGet(delta)
                        emitStats()
                    },
                    onBytesReceived = { delta ->
                        bytesReceived.addAndGet(delta)
                        emitStats()
                    },
                    onMessage = { event -> _incoming.emit(event) },
                    onClosed = {
                        peers.remove(remoteAddress)
                        emitStats()
                    },
                )
                peers[remoteAddress] = peer
                emitStats()
                peer.start(scope)
                logger.i("transport.$channelName.accept", "Accepted $remoteAddress")
            }
        }
    }

    suspend fun sendAll(message: T): SendAllResult {
        val snapshot = peers.values.toList()
        var success = 0
        var failure = 0
        snapshot.forEach { peer ->
            runCatching { peer.send(message) }
                .onSuccess { success += 1 }
                .onFailure { error ->
                    failure += 1
                    logger.w("transport.$channelName.send", "Failed to send to peer: ${error.message}")
                }
        }
        return SendAllResult(peerCount = snapshot.size, successCount = success, failureCount = failure)
    }


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

    fun connectionCount(): Int = peers.size

    fun bytesSent(): Long = bytesSent.get()

    fun bytesReceived(): Long = bytesReceived.get()

    override fun close() {
        acceptJob?.cancel()
        serverSocket?.close()
        peers.values.forEach(PeerConnection<T>::close)
        peers.clear()
        scope.cancel()
    }

    private fun emitStats() {
        _stats.tryEmit(
            TcpChannelStats(
                connectionCount = peers.size,
                bytesSent = bytesSent.get(),
                bytesReceived = bytesReceived.get(),
            ),
        )
    }
}

internal class TcpClientChannel<T>(
    private val host: String,
    private val port: Int,
    private val channelName: String,
    private val codec: MessageCodec<T>,
    private val logger: AppLogger,
) : Closeable {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val _incoming = MutableSharedFlow<TransportEvent<T>>(extraBufferCapacity = 64)
    val incoming: SharedFlow<TransportEvent<T>> = _incoming.asSharedFlow()
    private val _stats = MutableSharedFlow<TcpChannelStats>(replay = 1, extraBufferCapacity = 4)
    val stats: SharedFlow<TcpChannelStats> = _stats.asSharedFlow()

    private val bytesSent = AtomicLong(0)
    private val bytesReceived = AtomicLong(0)
    private var peer: PeerConnection<T>? = null

    fun connect(timeoutMs: Int = 2_500) {
        if (peer != null) return
        val socket = Socket()
        socket.connect(InetSocketAddress(host, port), timeoutMs)
        val remoteAddress = socket.remoteSocketAddress.toString()
        peer = PeerConnection(
            socket = socket,
            logger = logger,
            channelName = channelName,
            remoteAddress = remoteAddress,
            codec = codec,
            onBytesSent = { delta ->
                bytesSent.addAndGet(delta)
                emitStats()
            },
            onBytesReceived = { delta ->
                bytesReceived.addAndGet(delta)
                emitStats()
            },
            onMessage = { event -> _incoming.emit(event) },
            onClosed = {
                peer = null
                emitStats()
            },
        ).also {
            emitStats()
            it.start(scope)
            logger.i("transport.$channelName.connect", "Connected to $remoteAddress")
        }
    }

    suspend fun send(message: T) {
        peer?.send(message) ?: error("TCP $channelName channel is not connected")
    }

    fun isConnected(): Boolean = peer != null

    fun bytesSent(): Long = bytesSent.get()

    fun bytesReceived(): Long = bytesReceived.get()

    override fun close() {
        peer?.close()
        peer = null
        scope.cancel()
    }

    private fun emitStats() {
        _stats.tryEmit(
            TcpChannelStats(
                connectionCount = if (peer == null) 0 else 1,
                bytesSent = bytesSent.get(),
                bytesReceived = bytesReceived.get(),
            ),
        )
    }
}

private class PeerConnection<T>(
    private val socket: Socket,
    private val logger: AppLogger,
    private val channelName: String,
    private val remoteAddress: String,
    private val codec: MessageCodec<T>,
    private val onBytesSent: suspend (Long) -> Unit,
    private val onBytesReceived: suspend (Long) -> Unit,
    private val onMessage: suspend (TransportEvent<T>) -> Unit,
    private val onClosed: () -> Unit,
) : Closeable {
    private val writerMutex = Mutex()
    private var readJob: Job? = null
    @Volatile
    private var closed = false

    fun start(scope: CoroutineScope) {
        readJob = scope.launch {
            val input = DataInputStream(BufferedInputStream(socket.getInputStream()))
            try {
                while (true) {
                    val size = input.readInt()
                    val bytes = ByteArray(size)
                    input.readFully(bytes)
                    onBytesReceived(size.toLong() + Int.SIZE_BYTES)
                    onMessage(TransportEvent(remoteAddress = remoteAddress, message = codec.decode(bytes)))
                }
            } catch (_: EOFException) {
                logger.d("transport.$channelName.close", "Closed by $remoteAddress")
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                logger.w("transport.$channelName.read", "Read failed for $remoteAddress: ${error.message}")
            } finally {
                close()
            }
        }
    }

    suspend fun send(message: T) {
        writerMutex.withLock {
            val payload = codec.encode(message)
            val output = DataOutputStream(BufferedOutputStream(socket.getOutputStream()))
            output.writeInt(payload.size)
            output.write(payload)
            output.flush()
            onBytesSent(payload.size.toLong() + Int.SIZE_BYTES)
        }
    }

    override fun close() {
        if (closed) return
        closed = true
        val job = readJob
        readJob = null
        if (job != null && job.isActive) {
            job.cancel()
        }
        socket.close()
        onClosed()
    }
}

internal object ControlMessageCodec : MessageCodec<ControlMessage> by JsonMessageCodec(ControlMessage.serializer())

internal object SyncRequestPacketCodec :
    MessageCodec<SyncRequestPacket> by JsonMessageCodec(SyncRequestPacket.serializer())

internal object SyncResponsePacketCodec :
    MessageCodec<SyncResponsePacket> by JsonMessageCodec(SyncResponsePacket.serializer())
