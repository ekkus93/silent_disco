package com.ekkus.silentdisco.feature.diagnostics

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Share
import androidx.compose.material.icons.filled.Sync
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.TuningField
import com.ekkus.silentdisco.app.label
import com.ekkus.silentdisco.app.summary
import com.ekkus.silentdisco.core.audio.OboeBridge

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DiagnosticsScreen(
    uiState: AppUiState,
    onBack: () -> Unit,
    onManualResync: () -> Unit,
    onAdjustTuning: (TuningField, Int) -> Unit,
    onShare: (String) -> Unit,
) {
    val shareText = buildString {
        appendLine("Host session: ${uiState.hostDiagnostics.sessionId}")
        appendLine("Listeners: ${uiState.hostDiagnostics.connectedListenerCount}/${uiState.hostDiagnostics.listenerCount}")
        appendLine("Desynced listeners: ${uiState.hostDiagnostics.desyncedListenerCount}")
        appendLine("Packet sends: ${uiState.hostDiagnostics.packetSendCount}")
        appendLine("Packet budget: ${uiState.hostDiagnostics.packetBudgetSummary}")
        appendLine("Host metrics: ${uiState.hostDiagnostics.metricsSummary}")
        appendLine("Listener offset: ${uiState.listenerDiagnostics.hostOffsetMs} ms")
        appendLine("RTT: ${uiState.listenerDiagnostics.rttMs} ms")
        appendLine("Jitter: ${uiState.listenerDiagnostics.jitterMs} ms")
        appendLine("Buffer depth: ${uiState.listenerDiagnostics.bufferDepthMs} ms")
        appendLine("Packet loss: ${uiState.listenerDiagnostics.packetLossCount}")
        appendLine("Late drops: ${uiState.listenerDiagnostics.lateDropCount}")
        appendLine("Underruns: ${uiState.listenerDiagnostics.underrunCount}")
        appendLine("Invalid packets: ${uiState.listenerDiagnostics.invalidPacketCount}")
        appendLine("Concealed packets: ${uiState.listenerDiagnostics.concealedPacketCount}")
        appendLine("Listener metrics: ${uiState.listenerDiagnostics.metricsSummary}")
        appendLine("Tuning: ${uiState.tuningSettings.summary()}")
        appendLine("Last error: ${uiState.lastError ?: "none"}")
        appendLine("Audio backend: ${OboeBridge.backendSummary()}")
        appendLine("Audio status: ${OboeBridge.statusSummary()}")
    }
    Column(modifier = Modifier.fillMaxSize()) {
        TopAppBar(
            title = { Text("Diagnostics") },
            navigationIcon = {
                IconButton(onClick = onBack) {
                    Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                }
            },
        )
        Column(
            modifier = Modifier
                .weight(1f)
                .padding(16.dp)
                .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
        Card(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text("Host", style = MaterialTheme.typography.titleMedium)
                Text("Session id: ${uiState.hostDiagnostics.sessionId}")
                Text("Listener count: ${uiState.hostDiagnostics.listenerCount}")
                Text("Pending requests: ${uiState.hostDiagnostics.pendingJoinCount}")
                Text("Desynced listeners: ${uiState.hostDiagnostics.desyncedListenerCount}")
                Text("Packet send count: ${uiState.hostDiagnostics.packetSendCount}")
                Text("Send rate: ${uiState.hostDiagnostics.packetSendRatePerSecond} pkt/s")
                Text("Packet budget: ${uiState.hostDiagnostics.packetBudgetSummary}")
                Text("Stream state: ${uiState.hostDiagnostics.streamState.label()}")
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
                Text("Invalid packets: ${uiState.listenerDiagnostics.invalidPacketCount}")
                Text("Concealed packets: ${uiState.listenerDiagnostics.concealedPacketCount}")
                Text("Resync count: ${uiState.listenerDiagnostics.resyncCount}")
                Text("Playback state: ${uiState.listenerDiagnostics.playbackState}")
                Text("Playback position: ${uiState.listenerDiagnostics.playbackPositionMs} ms")
                Text("Reached EOF: ${uiState.listenerDiagnostics.endOfStreamReached}")
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
        Card(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text("Tuning", style = MaterialTheme.typography.titleMedium)
                Text("Current: ${uiState.tuningSettings.summary()}")
                TuningRow(
                    label = "Sync sample window",
                    value = "${uiState.tuningSettings.syncSampleWindow} samples",
                    onDecrease = { onAdjustTuning(TuningField.SyncSampleWindow, -1) },
                    onIncrease = { onAdjustTuning(TuningField.SyncSampleWindow, 1) },
                )
                TuningRow(
                    label = "Sync cadence",
                    value = "${uiState.tuningSettings.syncCadenceMs} ms",
                    onDecrease = { onAdjustTuning(TuningField.SyncCadenceMs, -1) },
                    onIncrease = { onAdjustTuning(TuningField.SyncCadenceMs, 1) },
                )
                TuningRow(
                    label = "Startup buffer",
                    value = "${uiState.tuningSettings.startupBufferMs} ms",
                    onDecrease = { onAdjustTuning(TuningField.StartupBufferMs, -1) },
                    onIncrease = { onAdjustTuning(TuningField.StartupBufferMs, 1) },
                )
                TuningRow(
                    label = "Late packet threshold",
                    value = "${uiState.tuningSettings.latePacketThresholdMs} ms",
                    onDecrease = { onAdjustTuning(TuningField.LatePacketThresholdMs, -1) },
                    onIncrease = { onAdjustTuning(TuningField.LatePacketThresholdMs, 1) },
                )
                TuningRow(
                    label = "Hard resync threshold",
                    value = "${uiState.tuningSettings.hardResyncThresholdMs} ms",
                    onDecrease = { onAdjustTuning(TuningField.HardResyncThresholdMs, -1) },
                    onIncrease = { onAdjustTuning(TuningField.HardResyncThresholdMs, 1) },
                )
                TuningRow(
                    label = "Sync drift threshold",
                    value = "${"%.1f".format(uiState.tuningSettings.syncDriftThresholdMs)} ms",
                    onDecrease = { onAdjustTuning(TuningField.SyncDriftThresholdMs, -1) },
                    onIncrease = { onAdjustTuning(TuningField.SyncDriftThresholdMs, 1) },
                )
            }
        }
        Button(onClick = onManualResync, modifier = Modifier.fillMaxWidth()) {
            Icon(Icons.Filled.Sync, contentDescription = null, modifier = Modifier.size(ButtonDefaults.IconSize))
            Spacer(Modifier.size(ButtonDefaults.IconSpacing))
            Text("Manual Resync")
        }
        Button(onClick = { onShare(shareText) }, modifier = Modifier.fillMaxWidth()) {
            Icon(Icons.Filled.Share, contentDescription = null, modifier = Modifier.size(ButtonDefaults.IconSize))
            Spacer(Modifier.size(ButtonDefaults.IconSpacing))
            Text("Share Debug Info")
        }
        }
    }
}

@Composable
private fun TuningRow(
    label: String,
    value: String,
    onDecrease: () -> Unit,
    onIncrease: () -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(label, style = MaterialTheme.typography.bodyLarge)
            Text(value, style = MaterialTheme.typography.bodyMedium)
        }
        Button(onClick = onDecrease) { Text("-") }
        Button(onClick = onIncrease) { Text("+") }
    }
}
