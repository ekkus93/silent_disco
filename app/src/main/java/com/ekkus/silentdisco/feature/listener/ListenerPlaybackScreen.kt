package com.ekkus.silentdisco.feature.listener

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.ExitToApp
import androidx.compose.material.icons.filled.BarChart
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlin.math.roundToInt
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.connectionQualitySummary
import com.ekkus.silentdisco.app.label
import com.ekkus.silentdisco.app.syncSummary
import com.ekkus.silentdisco.core.model.PlaybackState

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ListenerPlaybackScreen(
    uiState: AppUiState,
    onBack: () -> Unit,
    onVolumeChanged: (Float) -> Unit,
    onLeaveSession: () -> Unit,
    onReconnect: () -> Unit,
    onOpenDiagnostics: () -> Unit,
) {
    Column(modifier = Modifier.fillMaxSize()) {
        TopAppBar(
            title = { Text("Now Playing") },
            navigationIcon = {
                IconButton(onClick = onBack) {
                    Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                }
            },
        )
        Column(
            modifier = Modifier
                .weight(1f)
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("Session: ${uiState.selectedSession?.name ?: "Unknown"}")
                    Text("Host: ${uiState.selectedSession?.hostDeviceName ?: "Unknown"}")
                    Text("Sync quality: ${uiState.syncSummary()}")
                    Text("Connection quality: ${uiState.connectionQualitySummary()}")
                    Text("Playback state: ${uiState.listenerPlaybackState.label()}")
                    Text("Now playing: ${uiState.hostForm.selectedAudio?.displayName ?: "Host-selected stream"}")
                }
            }
            if (uiState.listenerPlaybackState == PlaybackState.BUFFERING) {
                LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
            }
            Text("Local volume — ${(uiState.localVolume * 100).roundToInt()}%")
            Slider(
                value = uiState.localVolume,
                onValueChange = onVolumeChanged,
                valueRange = 0f..1f,
            )
            Button(onClick = onOpenDiagnostics, modifier = Modifier.fillMaxWidth()) {
                Icon(Icons.Filled.BarChart, contentDescription = null, modifier = Modifier.size(ButtonDefaults.IconSize))
                Spacer(Modifier.size(ButtonDefaults.IconSpacing))
                Text("Diagnostics")
            }
            Button(onClick = onReconnect, modifier = Modifier.fillMaxWidth()) {
                Text("Reconnect")
            }
            Button(onClick = onLeaveSession, modifier = Modifier.fillMaxWidth()) {
                Icon(Icons.AutoMirrored.Filled.ExitToApp, contentDescription = null, modifier = Modifier.size(ButtonDefaults.IconSize))
                Spacer(Modifier.size(ButtonDefaults.IconSpacing))
                Text("Leave Session")
            }
        }
    }
}
