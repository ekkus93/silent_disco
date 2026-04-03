package com.ekkus.silentdisco.feature.host

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.core.model.JoinRequest

@Composable
fun HostControlScreen(
    uiState: AppUiState,
    onAddDemoJoinRequest: () -> Unit,
    onApprove: (JoinRequest) -> Unit,
    onReject: (JoinRequest) -> Unit,
    onTrust: (String) -> Unit,
    onRemove: (String) -> Unit,
    onStart: () -> Unit,
    onPause: () -> Unit,
    onStop: () -> Unit,
    onEndSession: () -> Unit,
    onOpenDiagnostics: () -> Unit,
) {
    LazyColumn(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        item {
            Text("Host Control", style = MaterialTheme.typography.headlineMedium)
        }
        item {
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("Session status", style = MaterialTheme.typography.titleMedium)
                    Text("Host state: ${uiState.hostState}")
                    Text("Playback: ${uiState.hostPlaybackState}")
                    Text("Health: ${uiState.hostDiagnostics.connectedListenerCount} connected / ${uiState.hostDiagnostics.pendingJoinCount} pending")
                    Text("Sync trouble: ${uiState.hostDiagnostics.desyncedListenerCount} listener(s)")
                }
            }
        }
        item {
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text("Selected audio", style = MaterialTheme.typography.titleMedium)
                    Text(uiState.hostForm.selectedAudio?.displayName ?: "No file selected")
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(onClick = onStart) { Text("Start") }
                        Button(onClick = onPause) { Text("Pause") }
                        Button(onClick = onStop) { Text("Stop") }
                    }
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(onClick = onAddDemoJoinRequest) { Text("Add Demo Join") }
                        Button(onClick = onOpenDiagnostics) { Text("Diagnostics") }
                        Button(onClick = onEndSession) { Text("End Session") }
                    }
                }
            }
        }
        item {
            Text("Pending join requests", style = MaterialTheme.typography.titleLarge)
        }
        items(uiState.pendingJoinRequests, key = { it.requestId }) { request ->
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(request.listenerName, style = MaterialTheme.typography.titleMedium)
                    Text("Request id: ${request.requestId.take(8)}")
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(onClick = { onApprove(request) }) { Text("Approve") }
                        Button(onClick = { onReject(request) }) { Text("Reject") }
                    }
                }
            }
        }
        item {
            Text("Approved / connected listeners", style = MaterialTheme.typography.titleLarge)
        }
        items(uiState.approvedListeners, key = { it.deviceId }) { listener ->
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(listener.displayName, style = MaterialTheme.typography.titleMedium)
                    Text("Join state: ${listener.joinState}")
                    Text("Transport: ${listener.connectionState}")
                    Text("Sync: ${listener.syncQuality}")
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(onClick = { onTrust(listener.deviceId) }) { Text("Trust") }
                        Button(onClick = { onRemove(listener.deviceId) }) { Text("Remove") }
                    }
                }
            }
        }
    }
}
