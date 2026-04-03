package com.ekkus.silentdisco.feature.listener

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.connectionQualitySummary
import com.ekkus.silentdisco.app.syncSummary

@Composable
fun ListenerPlaybackScreen(
    uiState: AppUiState,
    onVolumeChanged: (Float) -> Unit,
    onLeaveSession: () -> Unit,
    onReconnect: () -> Unit,
    onOpenDiagnostics: () -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text("Listener Playback", style = MaterialTheme.typography.headlineMedium)
        Card(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Session: ${uiState.selectedSession?.name ?: "Unknown"}")
                Text("Host: ${uiState.selectedSession?.hostDeviceName ?: "Unknown"}")
                Text("Sync quality: ${uiState.syncSummary()}")
                Text("Connection quality: ${uiState.connectionQualitySummary()}")
                Text("Playback state: ${uiState.listenerPlaybackState}")
                Text("Buffer depth: ${uiState.listenerDiagnostics.bufferDepthMs} ms")
                Text("Concealed packets: ${uiState.listenerDiagnostics.concealedPacketCount}")
                Text("EOF reached: ${uiState.listenerDiagnostics.endOfStreamReached}")
                Text("Now playing: ${uiState.hostForm.selectedAudio?.displayName ?: "Host-selected stream"}")
            }
        }
        Text("Local volume")
        Slider(
            value = uiState.localVolume,
            onValueChange = onVolumeChanged,
            valueRange = 0f..1f,
        )
        Button(onClick = onOpenDiagnostics, modifier = Modifier.fillMaxWidth()) {
            Text("Diagnostics")
        }
        Button(onClick = onReconnect, modifier = Modifier.fillMaxWidth()) {
            Text("Reconnect")
        }
        Button(onClick = onLeaveSession, modifier = Modifier.fillMaxWidth()) {
            Text("Leave Session")
        }
    }
}
