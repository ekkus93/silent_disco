package com.ekkus.silentdisco.core.diagnostics

import com.ekkus.silentdisco.core.model.HostDiagnosticsSnapshot
import com.ekkus.silentdisco.core.model.ListenerDiagnosticsSnapshot
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

class DiagnosticsStore {
    private val _hostDiagnostics = MutableStateFlow(HostDiagnosticsSnapshot())
    private val _listenerDiagnostics = MutableStateFlow(ListenerDiagnosticsSnapshot())

    val hostDiagnostics: StateFlow<HostDiagnosticsSnapshot> = _hostDiagnostics.asStateFlow()
    val listenerDiagnostics: StateFlow<ListenerDiagnosticsSnapshot> = _listenerDiagnostics.asStateFlow()

    fun updateHost(transform: (HostDiagnosticsSnapshot) -> HostDiagnosticsSnapshot) {
        _hostDiagnostics.value = transform(_hostDiagnostics.value)
    }

    fun updateListener(transform: (ListenerDiagnosticsSnapshot) -> ListenerDiagnosticsSnapshot) {
        _listenerDiagnostics.value = transform(_listenerDiagnostics.value)
    }
}
