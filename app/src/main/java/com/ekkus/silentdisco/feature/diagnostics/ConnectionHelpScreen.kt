package com.ekkus.silentdisco.feature.diagnostics

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.BugReport
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Share
import androidx.compose.material.icons.filled.Sync
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.SessionHealthLevel
import com.ekkus.silentdisco.app.canManualResync
import com.ekkus.silentdisco.app.hostSessionHealthSummary
import com.ekkus.silentdisco.app.listenerConnectionHealthSummary
import com.ekkus.silentdisco.core.model.AppRole
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ConnectionHelpScreen(
    uiState: AppUiState,
    onBack: () -> Unit,
    onResynchronize: () -> Unit,
    onReconnect: () -> Unit,
    onShareSupportReport: () -> Unit,
    onAdvancedDiagnostics: () -> Unit,
) {
    val hostContext = uiState.selectedRole == AppRole.HOST
    val summary = if (hostContext) {
        uiState.hostSessionHealthSummary()
    } else {
        uiState.listenerConnectionHealthSummary()
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .testTag("connection-help-screen"),
    ) {
        TopAppBar(
            title = { Text("Connection help") },
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
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(
                    modifier = Modifier.padding(20.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text(summary.title, style = MaterialTheme.typography.headlineSmall)
                    Text(summary.detail, style = MaterialTheme.typography.bodyLarge)
                }
            }

            Card(modifier = Modifier.fillMaxWidth()) {
                Column(
                    modifier = Modifier.padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Text("Status", style = MaterialTheme.typography.titleMedium)
                    if (hostContext) {
                        StatusRow(
                            "Session",
                            if (uiState.hostState.name == "ERROR") "Problem" else "Active",
                        )
                        StatusRow(
                            "Listeners",
                            if (uiState.hostDiagnostics.desyncedListenerCount > 0) {
                                "${uiState.hostDiagnostics.desyncedListenerCount} need attention"
                            } else {
                                "${uiState.hostDiagnostics.connectedListenerCount} connected"
                            },
                        )
                        StatusRow(
                            "Playback",
                            when (uiState.hostPlaybackState) {
                                PlaybackState.ERROR -> "Problem"
                                PlaybackState.PLAYING -> "Playing"
                                PlaybackState.PAUSED -> "Paused"
                                PlaybackState.BUFFERING -> "Preparing"
                                else -> "Stopped"
                            },
                        )
                    } else {
                        StatusRow(
                            "Connection",
                            when (uiState.listenerState) {
                                ListenerLifecycleState.DISCONNECTED -> "Lost"
                                ListenerLifecycleState.RECONNECTING -> "Recovering"
                                ListenerLifecycleState.ERROR -> "Problem"
                                ListenerLifecycleState.PLAYING -> "Good"
                                else -> "Updating"
                            },
                        )
                        StatusRow(
                            "Synchronization",
                            when (uiState.listenerState) {
                                ListenerLifecycleState.DESYNCED -> "Out of sync"
                                ListenerLifecycleState.SYNCING_CLOCK,
                                ListenerLifecycleState.BUFFERING,
                                -> "Synchronizing"
                                ListenerLifecycleState.PLAYING -> "Good"
                                else -> "Updating"
                            },
                        )
                        StatusRow(
                            "Audio",
                            when (uiState.listenerPlaybackState) {
                                PlaybackState.ERROR -> "Problem"
                                PlaybackState.BUFFERING -> "Buffering"
                                PlaybackState.PLAYING -> "Playing"
                                PlaybackState.PAUSED -> "Paused"
                                else -> "Stopped"
                            },
                        )
                    }
                }
            }

            if (!hostContext && uiState.canManualResync()) {
                Button(
                    onClick = onResynchronize,
                    modifier = Modifier
                        .fillMaxWidth()
                        .testTag("connection-help-resync"),
                ) {
                    Icon(Icons.Filled.Sync, contentDescription = null)
                    Text("Resynchronize audio")
                }
            }

            val canReconnect = !hostContext && uiState.listenerState in setOf(
                ListenerLifecycleState.DISCONNECTED,
                ListenerLifecycleState.RECONNECTING,
                ListenerLifecycleState.DESYNCED,
                ListenerLifecycleState.ERROR,
            )
            if (canReconnect) {
                Button(
                    onClick = onReconnect,
                    modifier = Modifier
                        .fillMaxWidth()
                        .testTag("connection-help-reconnect"),
                ) {
                    Icon(Icons.Filled.Refresh, contentDescription = null)
                    Text("Reconnect")
                }
            }

            Button(
                onClick = onShareSupportReport,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Icon(Icons.Filled.Share, contentDescription = null)
                Text("Share support report")
            }

            TextButton(
                onClick = onAdvancedDiagnostics,
                modifier = Modifier
                    .fillMaxWidth()
                    .testTag("connection-help-advanced"),
            ) {
                Icon(Icons.Filled.BugReport, contentDescription = null)
                Text("Advanced diagnostics")
            }

            if (summary.level == SessionHealthLevel.GOOD) {
                Text(
                    "No recovery action is needed right now.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
private fun StatusRow(label: String, value: String) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(label, style = MaterialTheme.typography.bodyLarge)
        Text(
            value,
            style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.primary,
        )
    }
}
