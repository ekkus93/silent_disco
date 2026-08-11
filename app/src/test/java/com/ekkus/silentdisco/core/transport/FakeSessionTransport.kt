package com.ekkus.silentdisco.core.transport

import com.ekkus.silentdisco.core.model.SessionInfo
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * A [SessionTransport] test double that records every call and returns a
 * configurable [TransportOperationResult]/exception, so a test can drive a
 * genuine Wi-Fi Direct failure path (distinct from a BLE failure) through
 * `MainViewModel`'s effect runner without a real `WifiP2pManager`.
 */
class FakeSessionTransport : SessionTransport {
    private val _snapshot = MutableStateFlow(TransportSnapshot())
    override val snapshot: StateFlow<TransportSnapshot> = _snapshot.asStateFlow()

    var startHostResult: TransportOperationResult = TransportOperationResult.Started
    var startHostFailure: Throwable? = null

    val startHostCalls = mutableListOf<SessionInfo>()
    var discoverPeersCallCount = 0
    val connectToSessionCalls = mutableListOf<SessionInfo>()
    var recordHeartbeatCallCount = 0
    val failCalls = mutableListOf<Pair<String, Boolean>>()
    var retryCallCount = 0
    var stopCallCount = 0

    override fun startHost(session: SessionInfo): TransportOperationResult {
        startHostCalls += session
        startHostFailure?.let { throw it }
        return startHostResult
    }

    override fun discoverPeers() {
        discoverPeersCallCount += 1
    }

    override fun connectToSession(session: SessionInfo) {
        connectToSessionCalls += session
    }

    override fun recordHeartbeat() {
        recordHeartbeatCallCount += 1
    }

    override fun fail(message: String, retryable: Boolean) {
        failCalls += message to retryable
    }

    override fun retry() {
        retryCallCount += 1
    }

    override fun stop() {
        stopCallCount += 1
    }

    /** Pushes a new snapshot, as the real transport would after an async platform callback. */
    fun emitSnapshot(snapshot: TransportSnapshot) {
        _snapshot.value = snapshot
    }
}
