package com.ekkus.silentdisco.feature.listener

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.listenerStateLabel

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
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text("Join / Approval / Connect", style = MaterialTheme.typography.headlineMedium)
        Text(session?.name ?: "No session selected")
        Card(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Current state: ${uiState.listenerStateLabel()}")
                Text("Discovered: ${uiState.connectionProgress.discovered}")
                Text("Requested: ${uiState.connectionProgress.requested}")
                Text("Approved: ${uiState.connectionProgress.approved}")
                Text("Connected: ${uiState.connectionProgress.connected}")
                Text("Synced: ${uiState.connectionProgress.synced}")
                Text("Playing: ${uiState.connectionProgress.playing}")
            }
        }
        if (session?.inviteCodeRequired == true) {
            OutlinedTextField(
                value = uiState.connectionProgress.inviteCode,
                onValueChange = onInviteCodeChanged,
                modifier = Modifier.fillMaxWidth(),
                label = { Text("Invite code") },
            )
        }
        Button(onClick = onJoin, modifier = Modifier.fillMaxWidth()) {
            Text("Request Join")
        }
        Button(onClick = onContinueWhenPlaying, modifier = Modifier.fillMaxWidth()) {
            Text("Continue to Playback")
        }
        Button(onClick = onRetry, modifier = Modifier.fillMaxWidth()) {
            Text("Retry")
        }
        Button(onClick = onCancel, modifier = Modifier.fillMaxWidth()) {
            Text("Cancel")
        }
    }
}
