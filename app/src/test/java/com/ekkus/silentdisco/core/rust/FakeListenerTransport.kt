package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.asSharedFlow

class FakeListenerTransport : ListenerTransport {
    private val mutableEvents = MutableSharedFlow<FfiListenerTransportEvent>(extraBufferCapacity = 16)
    override val events: Flow<FfiListenerTransportEvent> = mutableEvents.asSharedFlow()

    data class ConnectCall(
        val rawEndpoint: String,
        val localDeviceId: String,
        val localAddress: String,
    )

    val connectCalls = mutableListOf<ConnectCall>()
    val joinRequestCalls = mutableListOf<Pair<String, String?>>()
    val disconnectCalls = mutableListOf<String>()
    var closeCallCount: Int = 0
    var attachedPlayback: FfiListenerPlaybackHandle? = null
    var attachCallCount: Int = 0
    var detachCallCount: Int = 0

    override suspend fun connect(
        scope: CoroutineScope,
        rawEndpoint: String,
        localDeviceId: String,
        localAddress: String,
    ) {
        connectCalls += ConnectCall(rawEndpoint, localDeviceId, localAddress)
    }

    override suspend fun sendJoinRequest(displayName: String, inviteCode: String?) {
        joinRequestCalls += displayName to inviteCode
    }

    override suspend fun sendDisconnect(reason: String) {
        disconnectCalls += reason
    }

    override suspend fun sendSyncRequest(correlationId: ULong, localSendElapsedMs: ULong) = Unit

    override fun attachPlayback(playback: FfiListenerPlaybackHandle) {
        attachedPlayback = playback
        attachCallCount += 1
    }

    override fun detachPlayback() {
        attachedPlayback = null
        detachCallCount += 1
    }

    override suspend fun disconnect(reason: String) {
        disconnectCalls += reason
    }

    override fun close() {
        closeCallCount += 1
    }

    fun emit(event: FfiListenerTransportEvent) {
        mutableEvents.tryEmit(event)
    }
}
