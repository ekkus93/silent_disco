package com.ekkus.silentdisco.feature.home

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.permissionSummary

@Composable
fun HomeScreen(
    uiState: AppUiState,
    onRequestPermissions: () -> Unit,
    onHostClick: () -> Unit,
    onJoinClick: () -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text("Silent Disco PoC", style = MaterialTheme.typography.headlineMedium)
        Text(
            "Offline host/listener sync validation with BLE discovery, Wi-Fi Direct transport, and Oboe-oriented playback.",
            style = MaterialTheme.typography.bodyLarge,
        )

        Card(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Permissions", style = MaterialTheme.typography.titleMedium)
                Text(uiState.permissionSummary())
                Button(onClick = onRequestPermissions) {
                    Text("Grant / Refresh Permissions")
                }
            }
        }

        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Button(onClick = onHostClick, modifier = Modifier.weight(1f)) {
                Text("Host a Session")
            }
            Button(onClick = onJoinClick, modifier = Modifier.weight(1f)) {
                Text("Join a Session")
            }
        }
    }
}
