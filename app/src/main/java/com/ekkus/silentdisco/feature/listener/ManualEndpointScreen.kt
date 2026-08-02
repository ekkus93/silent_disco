package com.ekkus.silentdisco.feature.listener

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
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.core.model.ManualConnectUiState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.canConnect
import com.ekkus.silentdisco.core.model.isInProgress
import com.ekkus.silentdisco.ui.components.PrimaryProblemCard

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ManualEndpointScreen(
    uiState: AppUiState,
    onBack: () -> Unit,
    onInputChanged: (String) -> Unit,
    onInviteCodeChanged: (String) -> Unit,
    onConnect: () -> Unit,
    onCancel: () -> Unit,
) {
    val form = uiState.manualEndpointForm
    val connectState = uiState.manualConnectState
    val inProgress = connectState.isInProgress()

    Column(
        modifier = Modifier
            .fillMaxSize()
            .testTag("manual-endpoint-screen"),
    ) {
        TopAppBar(
            title = { Text("Connect manually") },
            navigationIcon = {
                IconButton(onClick = onBack) {
                    Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Cancel and go back")
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
                "Paste the connection payload copied from the desktop host.",
                style = MaterialTheme.typography.bodyLarge,
            )
            OutlinedTextField(
                value = form.rawInput,
                onValueChange = onInputChanged,
                modifier = Modifier
                    .fillMaxWidth()
                    .testTag("manual-endpoint-input"),
                label = { Text("Connection payload") },
                isError = form.validationError != null,
                supportingText = form.validationError?.let { message -> { Text(message) } },
                enabled = !inProgress,
                minLines = 3,
            )
            if (form.hostAddress != null && form.sessionId != null) {
                Card(modifier = Modifier.fillMaxWidth().testTag("manual-endpoint-preview")) {
                    Column(
                        modifier = Modifier.padding(16.dp),
                        verticalArrangement = Arrangement.spacedBy(4.dp),
                    ) {
                        Text("Host: ${form.hostAddress}", style = MaterialTheme.typography.bodyMedium)
                        Text("Session: ${form.sessionId}", style = MaterialTheme.typography.bodyMedium)
                        Text(
                            "Protocol version: ${form.protocolVersion}",
                            style = MaterialTheme.typography.bodyMedium,
                        )
                    }
                }
            }
            if (form.inviteCodeRequired) {
                OutlinedTextField(
                    value = form.inviteCode,
                    onValueChange = onInviteCodeChanged,
                    modifier = Modifier
                        .fillMaxWidth()
                        .testTag("manual-endpoint-invite-code"),
                    label = { Text("Invite code") },
                    enabled = !inProgress,
                    singleLine = true,
                )
            }

            when (connectState) {
                ManualConnectUiState.Idle -> Unit

                ManualConnectUiState.Connecting -> ManualConnectProgress("Connecting to host…")

                is ManualConnectUiState.AwaitingApproval -> ManualConnectProgress(
                    "Waiting for ${connectState.hostName} to approve " +
                        "\"${connectState.sessionName}\"…",
                )

                is ManualConnectUiState.Approved -> Card(
                    modifier = Modifier.fillMaxWidth().testTag("manual-endpoint-approved"),
                ) {
                    Column(modifier = Modifier.padding(16.dp)) {
                        Text(
                            "Connected",
                            style = MaterialTheme.typography.titleMedium,
                            modifier = Modifier.semantics { heading() },
                        )
                        Text(
                            "The host approved this device. Waiting for the host to start playback.",
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }

                is ManualConnectUiState.Streaming -> Card(
                    modifier = Modifier.fillMaxWidth().testTag("manual-endpoint-streaming"),
                ) {
                    Column(modifier = Modifier.padding(16.dp)) {
                        Text(
                            "Connected",
                            style = MaterialTheme.typography.titleMedium,
                            modifier = Modifier.semantics { heading() },
                        )
                        Text(
                            playbackStateLabel(connectState.playbackState),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }

                is ManualConnectUiState.Rejected -> PrimaryProblemCard(
                    title = "Host declined this connection",
                    detail = connectState.reason,
                    primaryActionLabel = "Try again",
                    onPrimaryAction = onCancel,
                    modifier = Modifier.testTag("manual-endpoint-problem"),
                )

                is ManualConnectUiState.Disconnected -> PrimaryProblemCard(
                    title = "Host disconnected",
                    detail = connectState.message ?: "The connection was closed.",
                    primaryActionLabel = "Try again",
                    onPrimaryAction = onCancel,
                    modifier = Modifier.testTag("manual-endpoint-problem"),
                )

                is ManualConnectUiState.Failed -> PrimaryProblemCard(
                    title = "Couldn’t connect",
                    detail = connectState.message,
                    primaryActionLabel = "Try again",
                    onPrimaryAction = onCancel,
                    modifier = Modifier.testTag("manual-endpoint-problem"),
                )
            }

            if (connectState == ManualConnectUiState.Idle) {
                Button(
                    onClick = onConnect,
                    enabled = form.canConnect(),
                    modifier = Modifier
                        .fillMaxWidth()
                        .testTag("manual-endpoint-connect"),
                ) {
                    Text("Connect")
                }
            }
            if (connectState !is ManualConnectUiState.Approved && connectState !is ManualConnectUiState.Streaming) {
                TextButton(
                    onClick = onCancel,
                    modifier = Modifier.fillMaxWidth().testTag("manual-endpoint-cancel"),
                ) {
                    Text(if (inProgress) "Cancel" else "Back")
                }
            }
        }
    }
}

internal fun playbackStateLabel(playbackState: PlaybackState): String = when (playbackState) {
    PlaybackState.BUFFERING -> "Buffering…"
    PlaybackState.PLAYING -> "Playing"
    PlaybackState.PAUSED -> "Paused by the host"
    PlaybackState.UNDERRUN -> "Playing (recovering from an underrun)"
    PlaybackState.STOPPED -> "Stopped"
    PlaybackState.READY, PlaybackState.ERROR -> "Streaming"
}

@Composable
private fun ManualConnectProgress(label: String) {
    Row(
        modifier = Modifier.fillMaxWidth().testTag("manual-endpoint-progress"),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        CircularProgressIndicator(
            modifier = Modifier.semantics { contentDescription = label },
        )
        Text(label, style = MaterialTheme.typography.bodyLarge)
    }
}
