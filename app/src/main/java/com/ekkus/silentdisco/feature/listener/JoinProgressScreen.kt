package com.ekkus.silentdisco.feature.listener

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.ConnectionProgressState
import com.ekkus.silentdisco.app.StepState
import com.ekkus.silentdisco.app.approvedStep
import com.ekkus.silentdisco.app.connectedStep
import com.ekkus.silentdisco.app.discoveredStep
import com.ekkus.silentdisco.app.listenerStateLabel
import com.ekkus.silentdisco.app.playingStep
import com.ekkus.silentdisco.app.requestedStep
import com.ekkus.silentdisco.app.syncedStep
import com.ekkus.silentdisco.core.model.ListenerLifecycleState

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun JoinProgressScreen(
    uiState: AppUiState,
    onInviteCodeChanged: (String) -> Unit,
    onJoin: () -> Unit,
    onCancel: () -> Unit,
    onRetry: () -> Unit,
    onContinueWhenPlaying: () -> Unit,
) {
    val session = uiState.selectedSession
    val state = uiState.listenerState
    val progress = uiState.connectionProgress

    Column(modifier = Modifier.fillMaxSize()) {
        TopAppBar(
            title = { Text("Join Session") },
            navigationIcon = {
                IconButton(onClick = onCancel) {
                    Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Cancel")
                }
            },
        )
        Column(
            modifier = Modifier
                .weight(1f)
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(session?.name ?: "No session selected", style = MaterialTheme.typography.titleMedium)

            Card(modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    ConnectionStepRow("Discovering session", progress.discoveredStep())
                    ConnectionStepRow("Sending join request", progress.requestedStep())
                    ConnectionStepRow("Awaiting host approval", progress.approvedStep())
                    ConnectionStepRow("Connecting transport", progress.connectedStep())
                    ConnectionStepRow("Syncing clock", progress.syncedStep())
                    ConnectionStepRow("Playing", progress.playingStep())
                }
            }

            Text(
                text = uiState.listenerStateLabel(),
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.Medium,
            )

            if (session?.inviteCodeRequired == true) {
                OutlinedTextField(
                    value = progress.inviteCode,
                    onValueChange = onInviteCodeChanged,
                    modifier = Modifier.fillMaxWidth(),
                    label = { Text("Invite code") },
                )
                Text(
                    "Enter the host's invite code before requesting access.",
                    style = MaterialTheme.typography.bodySmall,
                )
            }

            if (state in joinableStates) {
                Button(onClick = onJoin, modifier = Modifier.fillMaxWidth()) {
                    Text("Request Join")
                }
            }

            if (state == ListenerLifecycleState.PLAYING) {
                Button(onClick = onContinueWhenPlaying, modifier = Modifier.fillMaxWidth()) {
                    Text("Continue to Playback")
                }
            }

            if (state in retryableStates) {
                Button(onClick = onRetry, modifier = Modifier.fillMaxWidth()) {
                    Text("Retry")
                }
            }

            if (state != ListenerLifecycleState.PLAYING) {
                Button(onClick = onCancel, modifier = Modifier.fillMaxWidth()) {
                    Text("Cancel")
                }
            }
        }
    }
}

private val joinableStates = setOf(
    ListenerLifecycleState.IDLE,
    ListenerLifecycleState.SESSION_SELECTED,
    ListenerLifecycleState.JOIN_REQUESTED,
    ListenerLifecycleState.ERROR,
)

private val retryableStates = setOf(
    ListenerLifecycleState.ERROR,
    ListenerLifecycleState.DISCONNECTED,
)

@Composable
private fun ConnectionStepRow(label: String, stepState: StepState) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.fillMaxWidth(),
    ) {
        when (stepState) {
            StepState.Done -> Text(
                "✓",
                color = MaterialTheme.colorScheme.primary,
                style = MaterialTheme.typography.bodyLarge,
                modifier = Modifier.width(28.dp),
            )
            StepState.Active -> CircularProgressIndicator(
                modifier = Modifier
                    .size(18.dp)
                    .width(28.dp),
                strokeWidth = 2.dp,
            )
            StepState.Pending -> Text(
                "○",
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                style = MaterialTheme.typography.bodyLarge,
                modifier = Modifier.width(28.dp),
            )
        }
        Spacer(Modifier.width(8.dp))
        Text(
            label,
            style = MaterialTheme.typography.bodyMedium,
            fontWeight = if (stepState == StepState.Active) FontWeight.SemiBold else FontWeight.Normal,
            color = if (stepState == StepState.Pending) {
                MaterialTheme.colorScheme.onSurfaceVariant
            } else {
                MaterialTheme.colorScheme.onSurface
            },
        )
    }
}

