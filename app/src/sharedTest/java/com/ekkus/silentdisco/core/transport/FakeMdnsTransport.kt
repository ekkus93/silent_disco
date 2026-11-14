package com.ekkus.silentdisco.core.transport

import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow

class FakeMdnsTransport : MdnsTransport {
    private val sessions = MutableStateFlow<List<MdnsSessionAdvertisement>>(emptyList())
    override val discoveredSessions: StateFlow<List<MdnsSessionAdvertisement>> = sessions.asStateFlow()
    private val mutableFailures = MutableSharedFlow<String>(extraBufferCapacity = 8)
    override val failures: SharedFlow<String> = mutableFailures.asSharedFlow()

    var startResult: MdnsOperationResult = MdnsOperationResult.started()
    var startCallCount: Int = 0
    var stopCallCount: Int = 0

    override fun startDiscovery(): MdnsOperationResult {
        startCallCount += 1
        return startResult
    }

    override fun stopDiscovery() {
        stopCallCount += 1
        sessions.value = emptyList()
    }

    fun setSessions(value: List<MdnsSessionAdvertisement>) {
        sessions.value = value
    }

    fun fail(message: String) {
        mutableFailures.tryEmit(message)
    }
}
