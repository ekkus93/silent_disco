package com.ekkus.silentdisco.feature.listener

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.ExitToApp
import androidx.compose.material.icons.filled.Build
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.SessionHealthLevel
import com.ekkus.silentdisco.app.listenerConnectionHealthSummary
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.ui.components.StatusBadge
import com.ekkus.silentdisco.ui.components.StatusTone
import com.ekkus.silentdisco.ui.theme.WorkflowTouchTarget
import kotlin.math.roundToInt

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ListenerPlaybackV2Screen(
    uiState: AppUiState,
    onBackRequest: () -> Unit,
    onVolumeChanged: (Float) -> Unit,
    onFixConnection: () -> Unit,
    onLeaveRequest: () -> Unit,
) {
    val health = uiState.listenerConnectionHealthSummary()
    val actionUseful = health.level in setOf(SessionHealthLevel.ATTENTION, SessionHealthLevel.CRITICAL)
    val status = listenerPlaybackStatus(uiState)

    Column(
        modifier = Modifier
            .fillMaxSize()
            .testTag("listener-playback-screen"),
    ) {
        TopAppBar(
            title = { Text("Now playing") },
            navigationIcon = {
                IconButton(onClick = onBackRequest) {
                    Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                }
            },
        )
        ClientWorkflowStepper(stage = ClientWorkflowStage.LISTEN)
        Column(
            modifier = Modifier
                .weight(1f)
                .verticalScroll(rememberScrollState())
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(
                uiState.selectedSession?.name ?: "Silent Disco session",
                style = MaterialTheme.typography.headlineMedium,
            )
            Text(
                "Hosted by ${uiState.selectedSession?.displayHostName ?: "Unknown host"}",
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text(
                "The host controls playback",
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.primary,
            )

            Card(
                modifier = Modifier
                    .fillMaxWidth()
                    .semantics {
                        contentDescription = "$status. ${health.detail}"
                    }
                    .testTag("listener-playback-health"),
            ) {
                Column(
                    modifier = Modifier.padding(20.dp),
                    verticalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    if (
                        uiState.listenerPlaybackState == PlaybackState.BUFFERING ||
                        uiState.listenerState == ListenerLifecycleState.RECONNECTING
                    ) {
                        CircularProgressIndicator(
                            modifier = Modifier.semantics {
                                contentDescription = status
                            },
                        )
                    }
                    StatusBadge(
                        text = status,
                        tone = listenerPlaybackTone(uiState),
                    )
                    Text(
                        health.detail,
                        style = MaterialTheme.typography.bodyLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Text(
                        uiState.listenerCurrentTrackName ?: "Waiting for track information",
                        style = MaterialTheme.typography.titleMedium,
                        modifier = Modifier.testTag("listener-current-track"),
                    )
                }
            }

            Text(
                "Local volume — ${(uiState.localVolume * 100).roundToInt()}%",
                style = MaterialTheme.typography.titleMedium,
            )
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(min = WorkflowTouchTarget.minimum)
                    .semantics(mergeDescendants = true) {
                        contentDescription = "Local playback volume"
                    }
                    .testTag("listener-volume"),
                contentAlignment = Alignment.Center,
            ) {
                Slider(
                    value = uiState.localVolume,
                    onValueChange = onVolumeChanged,
                    valueRange = 0f..1f,
                    modifier = Modifier.fillMaxWidth(),
                )
            }

        }
        HorizontalDivider()
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            if (actionUseful) {
                Button(
                    onClick = onFixConnection,
                    modifier = Modifier
                        .fillMaxWidth()
                        .heightIn(min = WorkflowTouchTarget.minimum)
                        .testTag("listener-fix-connection"),
                ) {
                    Icon(Icons.Filled.Build, contentDescription = null)
                    Text("Fix connection")
                }
            }
            TextButton(
                onClick = onLeaveRequest,
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(min = WorkflowTouchTarget.minimum)
                    .testTag("listener-leave"),
            ) {
                Icon(Icons.AutoMirrored.Filled.ExitToApp, contentDescription = null)
                Text("Leave session")
            }
        }
    }
}

internal fun listenerPlaybackStatus(uiState: AppUiState): String = when {
    uiState.listenerPlaybackState == PlaybackState.ERROR -> "Playback problem"
    uiState.listenerState == ListenerLifecycleState.DISCONNECTED -> "Connection lost"
    uiState.listenerState == ListenerLifecycleState.DESYNCED -> "Audio is out of sync"
    uiState.listenerState == ListenerLifecycleState.RECONNECTING -> "Reconnecting"
    uiState.listenerPlaybackState == PlaybackState.BUFFERING -> "Buffering"
    uiState.listenerPlaybackState == PlaybackState.PAUSED -> "Playback paused"
    uiState.listenerPlaybackState == PlaybackState.STOPPED -> "Playback stopped"
    uiState.listenerState == ListenerLifecycleState.PLAYING &&
        uiState.listenerPlaybackState == PlaybackState.PLAYING -> "Playing in sync"
    else -> "Connection status is updating"
}

internal fun listenerPlaybackTone(uiState: AppUiState): StatusTone = when {
    uiState.listenerPlaybackState == PlaybackState.ERROR ||
        uiState.listenerState == ListenerLifecycleState.DISCONNECTED -> StatusTone.CRITICAL

    uiState.listenerState == ListenerLifecycleState.DESYNCED -> StatusTone.ATTENTION
    uiState.listenerState == ListenerLifecycleState.RECONNECTING ||
        uiState.listenerPlaybackState == PlaybackState.BUFFERING -> StatusTone.IN_PROGRESS

    uiState.listenerState == ListenerLifecycleState.PLAYING &&
        uiState.listenerPlaybackState == PlaybackState.PLAYING -> StatusTone.POSITIVE

    else -> StatusTone.NEUTRAL
}
