package com.ekkus.silentdisco.feature.diagnostics

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.core.audio.OboeBridge

@Composable
fun DiagnosticsScreen(
    uiState: AppUiState,
    onManualResync: () -> Unit,
    onShare: (String) -> Unit,
) {
    val shareText = buildString {
        appendLine("Host session: ${uiState.hostDiagnostics.sessionId}")
        appendLine("Listeners: ${uiState.hostDiagnostics.connectedListenerCount}/${uiState.hostDiagnostics.listenerCount}")
        appendLine("Packet sends: ${uiState.hostDiagnostics.packetSendCount}")
        appendLine("Listener offset: ${uiState.listenerDiagnostics.hostOffsetMs} ms")
        appendLine("RTT: ${uiState.listenerDiagnostics.rttMs} ms")
        appendLine("Jitter: ${uiState.listenerDiagnostics.jitterMs} ms")
        appendLine("Buffer depth: ${uiState.listenerDiagnostics.bufferDepthMs} ms")
        appendLine("Packet loss: ${uiState.listenerDiagnostics.packetLossCount}")
        appendLine("Underruns: ${uiState.listenerDiagnostics.underrunCount}")
        appendLine("Audio backend: ${OboeBridge.backendSummary()}")
        appendLine("Audio status: ${OboeBridge.statusSummary()}")
    }
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp)
            .verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text("Diagnostics", style = MaterialTheme.typography.headlineMedium)
        Card(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text("Host", style = MaterialTheme.typography.titleMedium)
                Text("Session id: ${uiState.hostDiagnostics.sessionId}")
                Text("Listener count: ${uiState.hostDiagnostics.listenerCount}")
                Text("Pending requests: ${uiState.hostDiagnostics.pendingJoinCount}")
                Text("Packet send count: ${uiState.hostDiagnostics.packetSendCount}")
                Text("Send rate: ${uiState.hostDiagnostics.packetSendRatePerSecond} pkt/s")
                Text("Stream state: ${uiState.hostDiagnostics.streamState}")
                Text("Last contact: ${uiState.hostDiagnostics.lastContactElapsedMs ?: -1} ms")
                Text("Metrics: ${uiState.hostDiagnostics.metricsSummary}")
            }
        }
        Card(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text("Listener", style = MaterialTheme.typography.titleMedium)
                Text("Offset estimate: ${uiState.listenerDiagnostics.hostOffsetMs} ms")
                Text("RTT: ${uiState.listenerDiagnostics.rttMs} ms")
                Text("Jitter: ${uiState.listenerDiagnostics.jitterMs} ms")
                Text("Buffer depth: ${uiState.listenerDiagnostics.bufferDepthMs} ms")
                Text("Packet loss: ${uiState.listenerDiagnostics.packetLossCount}")
                Text("Late drops: ${uiState.listenerDiagnostics.lateDropCount}")
                Text("Underruns: ${uiState.listenerDiagnostics.underrunCount}")
                Text("Resync count: ${uiState.listenerDiagnostics.resyncCount}")
                Text("Playback state: ${uiState.listenerDiagnostics.playbackState}")
                Text("Playback position: ${uiState.listenerDiagnostics.playbackPositionMs} ms")
                Text("Metrics: ${uiState.listenerDiagnostics.metricsSummary}")
            }
        }
        Card(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text("Audio engine", style = MaterialTheme.typography.titleMedium)
                Text("Backend: ${OboeBridge.backendSummary()}")
                Text("Status: ${OboeBridge.statusSummary()}")
            }
        }
        Button(onClick = onManualResync, modifier = Modifier.fillMaxWidth()) {
            Text("Manual Resync")
        }
        Button(onClick = { onShare(shareText) }, modifier = Modifier.fillMaxWidth()) {
            Text("Share Debug Info")
        }
    }
}
