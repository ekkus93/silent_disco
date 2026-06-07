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
import com.ekkus.silentdisco.BuildConfig
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.label
import com.ekkus.silentdisco.core.model.JoinRequest
import com.ekkus.silentdisco.core.model.PlaybackState

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
                    Text("Host state: ${uiState.hostState.label()}")
                    Text("Playback: ${uiState.hostPlaybackState.label()}")
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
                    val isPlaying = uiState.hostPlaybackState == PlaybackState.PLAYING
                    val isStopped = uiState.hostPlaybackState == PlaybackState.STOPPED
                    val hasAudio = uiState.hostForm.selectedAudio != null
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(onClick = onStart, enabled = !isPlaying && hasAudio) { Text("Start") }
                        Button(onClick = onPause, enabled = isPlaying) { Text("Pause") }
                        Button(onClick = onStop, enabled = !isStopped) { Text("Stop") }
                    }
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(onClick = onOpenDiagnostics) { Text("Diagnostics") }
                        Button(onClick = onEndSession) { Text("End Session") }
                    }
                    if (BuildConfig.DEBUG) {
                        Button(onClick = onAddDemoJoinRequest) { Text("[Debug] Add Demo Join") }
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
                    Text("Join state: ${listener.joinState.label()}")
                    Text("Transport: ${listener.connectionState.label()}")
                    Text("Sync: ${listener.syncQuality.label()}")
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(onClick = { onTrust(listener.deviceId) }) { Text("Trust") }
                        Button(onClick = { onRemove(listener.deviceId) }) { Text("Remove") }
                    }
                }
            }
        }
    }
}
