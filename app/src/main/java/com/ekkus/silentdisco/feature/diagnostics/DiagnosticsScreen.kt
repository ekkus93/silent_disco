package com.ekkus.silentdisco.feature.diagnostics

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.ExpandLess
import androidx.compose.material.icons.filled.ExpandMore
import androidx.compose.material.icons.filled.Share
import androidx.compose.material.icons.filled.Sync
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.TuningField
import com.ekkus.silentdisco.app.TuningSettings
import com.ekkus.silentdisco.app.canManualResync
import com.ekkus.silentdisco.app.hostSessionHealthSummary
import com.ekkus.silentdisco.app.label
import com.ekkus.silentdisco.app.listenerConnectionHealthSummary
import com.ekkus.silentdisco.app.summary
import com.ekkus.silentdisco.core.audio.OboeBridge
import com.ekkus.silentdisco.core.model.AppRole
import com.ekkus.silentdisco.ui.components.PrimaryProblemCard

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DiagnosticsScreen(
    uiState: AppUiState,
    onBack: () -> Unit,
    onManualResync: () -> Unit,
    onAdjustTuning: (TuningField, Int) -> Unit,
    onShare: (String) -> Unit,
    initialExpertExpanded: Boolean = false,
    initialExpertEnabled: Boolean = false,
) {
    var hostExpanded by rememberSaveable { mutableStateOf(false) }
    var listenerExpanded by rememberSaveable { mutableStateOf(false) }
    var outputExpanded by rememberSaveable { mutableStateOf(false) }
    var expertExpanded by rememberSaveable(initialExpertExpanded) { mutableStateOf(initialExpertExpanded) }
    var expertEnabled by rememberSaveable(initialExpertEnabled) { mutableStateOf(initialExpertEnabled) }
    val showHost = uiState.selectedRole == AppRole.HOST || uiState.hostDiagnostics.sessionId.isNotBlank()
    val showListener = uiState.selectedRole == AppRole.LISTENER || uiState.listenerDiagnostics.sessionId.isNotBlank()
    val hostHealth = uiState.hostSessionHealthSummary()
    val listenerHealth = uiState.listenerConnectionHealthSummary()
    val shareSummary = diagnosticsShareSummary(uiState)

    Column(
        modifier = Modifier
            .fillMaxSize()
            .testTag("advanced-diagnostics-screen"),
    ) {
        TopAppBar(
            title = { Text("Advanced diagnostics") },
            navigationIcon = {
                IconButton(onClick = onBack) {
                    Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                }
            },
        )
        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(
                "Technical connection, synchronization, and playback details for troubleshooting.",
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            uiState.lastError?.takeIf(String::isNotBlank)?.let { error ->
                PrimaryProblemCard(
                    title = "A recent operation failed",
                    detail = error,
                    primaryActionLabel = "Share support report",
                    onPrimaryAction = { onShare(shareSummary) },
                    modifier = Modifier.testTag("advanced-persistent-problem"),
                )
            }

            if (showHost) {
                SummaryCard(
                    title = "Host session",
                    result = hostHealth.title,
                    detail = hostHealth.detail,
                )
                ExpandableDiagnosticCard(
                    title = "Host technical details",
                    expanded = hostExpanded,
                    onToggle = { hostExpanded = !hostExpanded },
                ) {
                    DiagnosticValue("Listener count", uiState.hostDiagnostics.listenerCount.toString())
                    DiagnosticValue("Pending requests", uiState.hostDiagnostics.pendingJoinCount.toString())
                    DiagnosticValue("Connected listeners", uiState.hostDiagnostics.connectedListenerCount.toString())
                    DiagnosticValue("Desynced listeners", uiState.hostDiagnostics.desyncedListenerCount.toString())
                    DiagnosticValue("Packet send count", uiState.hostDiagnostics.packetSendCount.toString())
                    DiagnosticValue("Send rate", "${uiState.hostDiagnostics.packetSendRatePerSecond} pkt/s")
                    DiagnosticValue("Packet budget", uiState.hostDiagnostics.packetBudgetSummary)
                    DiagnosticValue("Stream state", uiState.hostDiagnostics.streamState.label())
                    DiagnosticValue("Last contact", "${uiState.hostDiagnostics.lastContactElapsedMs ?: -1} ms")
                    DiagnosticValue("Metrics", uiState.hostDiagnostics.metricsSummary)
                    DiagnosticValue("Last error", uiState.hostDiagnostics.lastError ?: "none")
                }
            }

            if (showListener) {
                SummaryCard(
                    title = "Listener connection",
                    result = listenerHealth.title,
                    detail = listenerHealth.detail,
                )
                ExpandableDiagnosticCard(
                    title = "Listener technical details",
                    expanded = listenerExpanded,
                    onToggle = { listenerExpanded = !listenerExpanded },
                ) {
                    DiagnosticValue("Offset estimate", "${uiState.listenerDiagnostics.hostOffsetMs} ms")
                    DiagnosticValue("RTT", "${uiState.listenerDiagnostics.rttMs} ms")
                    DiagnosticValue("Jitter", "${uiState.listenerDiagnostics.jitterMs} ms")
                    DiagnosticValue("Buffer depth", "${uiState.listenerDiagnostics.bufferDepthMs} ms")
                    DiagnosticValue("Packet loss", uiState.listenerDiagnostics.packetLossCount.toString())
                    DiagnosticValue("Late drops", uiState.listenerDiagnostics.lateDropCount.toString())
                    DiagnosticValue("Underruns", uiState.listenerDiagnostics.underrunCount.toString())
                    DiagnosticValue("Invalid packets", uiState.listenerDiagnostics.invalidPacketCount.toString())
                    DiagnosticValue("Concealed packets", uiState.listenerDiagnostics.concealedPacketCount.toString())
                    DiagnosticValue("Reconnect count", uiState.listenerDiagnostics.reconnectCount.toString())
                    DiagnosticValue("Resync count", uiState.listenerDiagnostics.resyncCount.toString())
                    DiagnosticValue("Playback state", uiState.listenerDiagnostics.playbackState.label())
                    DiagnosticValue("Playback position", "${uiState.listenerDiagnostics.playbackPositionMs} ms")
                    DiagnosticValue("Reached end of stream", uiState.listenerDiagnostics.endOfStreamReached.toString())
                    DiagnosticValue("Metrics", uiState.listenerDiagnostics.metricsSummary)
                    DiagnosticValue("Last error", uiState.listenerDiagnostics.lastError ?: "none")
                }
            }

            ExpandableDiagnosticCard(
                title = "Playback output",
                expanded = outputExpanded,
                onToggle = { outputExpanded = !outputExpanded },
            ) {
                DiagnosticValue("Output", "Android AudioTrack")
                DiagnosticValue("Native bridge availability", OboeBridge.backendSummary())
                DiagnosticValue("Native bridge status", OboeBridge.statusSummary())
            }

            Card(modifier = Modifier.fillMaxWidth()) {
                Column(
                    modifier = Modifier.padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    TextButton(
                        onClick = { expertExpanded = !expertExpanded },
                        modifier = Modifier
                            .fillMaxWidth()
                            .testTag("expert-tuning-toggle"),
                    ) {
                        Icon(
                            if (expertExpanded) Icons.Filled.ExpandLess else Icons.Filled.ExpandMore,
                            contentDescription = null,
                        )
                        Text("Expert tuning")
                    }
                    if (expertExpanded) {
                        Text(
                            "Changing these values can make synchronization worse.",
                            color = MaterialTheme.colorScheme.error,
                            style = MaterialTheme.typography.bodyMedium,
                        )
                        Text("Current: ${uiState.tuningSettings.summary()}")
                        if (!expertEnabled) {
                            Button(
                                onClick = { expertEnabled = true },
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .testTag("enable-expert-tuning"),
                            ) {
                                Text("Enable expert controls")
                            }
                        }
                        TuningRow(
                            "Sync sample window",
                            "${uiState.tuningSettings.syncSampleWindow} samples",
                            expertEnabled,
                            { onAdjustTuning(TuningField.SyncSampleWindow, -1) },
                            { onAdjustTuning(TuningField.SyncSampleWindow, 1) },
                        )
                        TuningRow(
                            "Sync cadence",
                            "${uiState.tuningSettings.syncCadenceMs} ms",
                            expertEnabled,
                            { onAdjustTuning(TuningField.SyncCadenceMs, -1) },
                            { onAdjustTuning(TuningField.SyncCadenceMs, 1) },
                        )
                        TuningRow(
                            "Startup buffer",
                            "${uiState.tuningSettings.startupBufferMs} ms",
                            expertEnabled,
                            { onAdjustTuning(TuningField.StartupBufferMs, -1) },
                            { onAdjustTuning(TuningField.StartupBufferMs, 1) },
                        )
                        TuningRow(
                            "Late packet threshold",
                            "${uiState.tuningSettings.latePacketThresholdMs} ms",
                            expertEnabled,
                            { onAdjustTuning(TuningField.LatePacketThresholdMs, -1) },
                            { onAdjustTuning(TuningField.LatePacketThresholdMs, 1) },
                        )
                        TuningRow(
                            "Hard resync threshold",
                            "${uiState.tuningSettings.hardResyncThresholdMs} ms",
                            expertEnabled,
                            { onAdjustTuning(TuningField.HardResyncThresholdMs, -1) },
                            { onAdjustTuning(TuningField.HardResyncThresholdMs, 1) },
                        )
                        TuningRow(
                            "Sync drift threshold",
                            "${"%.1f".format(uiState.tuningSettings.syncDriftThresholdMs)} ms",
                            expertEnabled,
                            { onAdjustTuning(TuningField.SyncDriftThresholdMs, -1) },
                            { onAdjustTuning(TuningField.SyncDriftThresholdMs, 1) },
                        )
                        OutlinedButton(
                            onClick = { onAdjustTuning(TuningField.ResetDefaults, 1) },
                            enabled = expertEnabled && uiState.tuningSettings != TuningSettings(),
                            modifier = Modifier
                                .fillMaxWidth()
                                .testTag("reset-tuning-defaults"),
                        ) {
                            Text("Reset tuning to defaults")
                        }
                    }
                }
            }

            val canResynchronize = uiState.canManualResync()
            Button(
                onClick = onManualResync,
                enabled = canResynchronize,
                modifier = Modifier
                    .fillMaxWidth()
                    .testTag("advanced-resynchronize"),
            ) {
                Icon(Icons.Filled.Sync, contentDescription = null)
                Text("Resynchronize audio")
            }
            if (!canResynchronize) {
                Text(
                    "Join a session before requesting audio resynchronization.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            OutlinedButton(
                onClick = { onShare(shareSummary) },
                modifier = Modifier
                    .fillMaxWidth()
                    .testTag("advanced-share-report"),
            ) {
                Icon(Icons.Filled.Share, contentDescription = null)
                Text("Share support report")
            }
        }
    }
}

@Composable
private fun SummaryCard(
    title: String,
    result: String,
    detail: String,
) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Text(title, style = MaterialTheme.typography.titleMedium)
            Text(result, style = MaterialTheme.typography.headlineSmall)
            Text(detail, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

@Composable
private fun ExpandableDiagnosticCard(
    title: String,
    expanded: Boolean,
    onToggle: () -> Unit,
    content: @Composable ColumnScope.() -> Unit,
) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            TextButton(onClick = onToggle, modifier = Modifier.fillMaxWidth()) {
                Icon(
                    if (expanded) Icons.Filled.ExpandLess else Icons.Filled.ExpandMore,
                    contentDescription = null,
                )
                Text(title)
            }
            if (expanded) content()
        }
    }
}

@Composable
private fun DiagnosticValue(label: String, value: String) {
    Text("$label: ${value.ifBlank { "not available" }}", style = MaterialTheme.typography.bodyMedium)
}

@Composable
private fun TuningRow(
    label: String,
    value: String,
    enabled: Boolean,
    onDecrease: () -> Unit,
    onIncrease: () -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(label, style = MaterialTheme.typography.bodyLarge)
            Text(value, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Button(onClick = onDecrease, enabled = enabled) { Text("−") }
        Button(onClick = onIncrease, enabled = enabled) { Text("+") }
    }
}

internal fun diagnosticsShareSummary(uiState: AppUiState): String = buildString {
    appendLine("Silent Disco diagnostics summary")
    appendLine("Host status: ${uiState.hostSessionHealthSummary().title}")
    appendLine("Listener status: ${uiState.listenerConnectionHealthSummary().title}")
    appendLine("Connected listeners: ${uiState.hostDiagnostics.connectedListenerCount}")
    appendLine("Desynced listeners: ${uiState.hostDiagnostics.desyncedListenerCount}")
    appendLine("Listener playback: ${uiState.listenerDiagnostics.playbackState.label()}")
    appendLine("Listener reconnects: ${uiState.listenerDiagnostics.reconnectCount}")
    appendLine("Listener resyncs: ${uiState.listenerDiagnostics.resyncCount}")
    appendLine("Tuning: ${uiState.tuningSettings.summary()}")
    appendLine("Identifiers, invite codes, and file paths are omitted.")
}
