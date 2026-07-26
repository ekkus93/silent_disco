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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.SessionHealthLevel
import com.ekkus.silentdisco.app.canManualResync
import com.ekkus.silentdisco.app.hostSessionHealthSummary
import com.ekkus.silentdisco.app.listenerConnectionHealthSummary
import com.ekkus.silentdisco.core.model.AppRole
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.ui.components.StatusBadge
import com.ekkus.silentdisco.ui.components.StatusTone

internal data class ConnectionIndicator(
    val label: String,
    val value: String,
    val tone: StatusTone,
)

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
    val indicators = connectionHelpIndicators(uiState, hostContext)

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
                    verticalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    Text(
                        summary.title,
                        modifier = Modifier.semantics { heading() },
                        style = MaterialTheme.typography.headlineSmall,
                    )
                    StatusBadge(
                        text = summary.title,
                        tone = summary.level.toStatusTone(),
                        semanticLabel = "Overall status: ${summary.title}",
                    )
                    Text(summary.detail, style = MaterialTheme.typography.bodyLarge)
                }
            }

            Card(modifier = Modifier.fillMaxWidth()) {
                Column(
                    modifier = Modifier.padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Text(
                        "Status",
                        modifier = Modifier.semantics { heading() },
                        style = MaterialTheme.typography.titleMedium,
                    )
                    indicators.forEach { indicator ->
                        StatusRow(indicator)
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
private fun StatusRow(indicator: ConnectionIndicator) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(indicator.label, style = MaterialTheme.typography.bodyLarge)
        StatusBadge(
            text = indicator.value,
            tone = indicator.tone,
            semanticLabel = "${indicator.label}: ${indicator.value}",
        )
    }
}

internal fun connectionHelpIndicators(
    uiState: AppUiState,
    hostContext: Boolean,
): List<ConnectionIndicator> = if (hostContext) {
    listOf(
        ConnectionIndicator(
            label = "Session",
            value = if (uiState.hostState == HostLifecycleState.ERROR) "Problem" else "Active",
            tone = if (uiState.hostState == HostLifecycleState.ERROR) StatusTone.CRITICAL else StatusTone.POSITIVE,
        ),
        ConnectionIndicator(
            label = "Listeners",
            value = if (uiState.hostDiagnostics.desyncedListenerCount > 0) {
                "${uiState.hostDiagnostics.desyncedListenerCount} need attention"
            } else {
                "${uiState.hostDiagnostics.connectedListenerCount} connected"
            },
            tone = if (uiState.hostDiagnostics.desyncedListenerCount > 0) {
                StatusTone.ATTENTION
            } else {
                StatusTone.POSITIVE
            },
        ),
        ConnectionIndicator(
            label = "Playback",
            value = when (uiState.hostPlaybackState) {
                PlaybackState.ERROR -> "Problem"
                PlaybackState.PLAYING -> "Playing"
                PlaybackState.PAUSED -> "Paused"
                PlaybackState.BUFFERING -> "Preparing"
                else -> "Stopped"
            },
            tone = when (uiState.hostPlaybackState) {
                PlaybackState.ERROR -> StatusTone.CRITICAL
                PlaybackState.PLAYING -> StatusTone.POSITIVE
                PlaybackState.BUFFERING -> StatusTone.IN_PROGRESS
                PlaybackState.PAUSED,
                PlaybackState.STOPPED,
                PlaybackState.READY,
                PlaybackState.UNDERRUN -> StatusTone.NEUTRAL
            },
        ),
    )
} else {
    listOf(
        ConnectionIndicator(
            label = "Connection",
            value = when (uiState.listenerState) {
                ListenerLifecycleState.DISCONNECTED -> "Lost"
                ListenerLifecycleState.RECONNECTING -> "Recovering"
                ListenerLifecycleState.ERROR -> "Problem"
                ListenerLifecycleState.PLAYING -> "Good"
                else -> "Updating"
            },
            tone = when (uiState.listenerState) {
                ListenerLifecycleState.DISCONNECTED,
                ListenerLifecycleState.ERROR -> StatusTone.CRITICAL
                ListenerLifecycleState.RECONNECTING -> StatusTone.IN_PROGRESS
                ListenerLifecycleState.PLAYING -> StatusTone.POSITIVE
                else -> StatusTone.NEUTRAL
            },
        ),
        ConnectionIndicator(
            label = "Synchronization",
            value = when (uiState.listenerState) {
                ListenerLifecycleState.DESYNCED -> "Out of sync"
                ListenerLifecycleState.SYNCING_CLOCK,
                ListenerLifecycleState.BUFFERING -> "Synchronizing"
                ListenerLifecycleState.PLAYING -> "Good"
                else -> "Updating"
            },
            tone = when (uiState.listenerState) {
                ListenerLifecycleState.DESYNCED -> StatusTone.ATTENTION
                ListenerLifecycleState.SYNCING_CLOCK,
                ListenerLifecycleState.BUFFERING -> StatusTone.IN_PROGRESS
                ListenerLifecycleState.PLAYING -> StatusTone.POSITIVE
                else -> StatusTone.NEUTRAL
            },
        ),
        ConnectionIndicator(
            label = "Audio",
            value = when (uiState.listenerPlaybackState) {
                PlaybackState.ERROR -> "Problem"
                PlaybackState.BUFFERING -> "Buffering"
                PlaybackState.PLAYING -> "Playing"
                PlaybackState.PAUSED -> "Paused"
                else -> "Stopped"
            },
            tone = when (uiState.listenerPlaybackState) {
                PlaybackState.ERROR -> StatusTone.CRITICAL
                PlaybackState.BUFFERING -> StatusTone.IN_PROGRESS
                PlaybackState.PLAYING -> StatusTone.POSITIVE
                PlaybackState.PAUSED,
                PlaybackState.STOPPED,
                PlaybackState.READY,
                PlaybackState.UNDERRUN -> StatusTone.NEUTRAL
            },
        ),
    )
}

private fun SessionHealthLevel.toStatusTone(): StatusTone = when (this) {
    SessionHealthLevel.GOOD -> StatusTone.POSITIVE
    SessionHealthLevel.ATTENTION -> StatusTone.ATTENTION
    SessionHealthLevel.CRITICAL -> StatusTone.CRITICAL
    SessionHealthLevel.UNKNOWN -> StatusTone.NEUTRAL
}
