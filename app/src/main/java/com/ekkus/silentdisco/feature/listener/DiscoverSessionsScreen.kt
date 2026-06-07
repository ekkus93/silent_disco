package com.ekkus.silentdisco.feature.listener

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.label
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.SessionInfo

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DiscoverSessionsScreen(
    uiState: AppUiState,
    onBack: () -> Unit,
    onRefresh: () -> Unit,
    onSelectSession: (SessionInfo) -> Unit,
) {
    val isScanning = uiState.listenerState == ListenerLifecycleState.SCANNING

    Column(modifier = Modifier.fillMaxSize()) {
        TopAppBar(
            title = { Text("Nearby Sessions") },
            navigationIcon = {
                IconButton(onClick = onBack) {
                    Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                }
            },
        )
        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            contentPadding = PaddingValues(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            item {
                Button(
                    onClick = onRefresh,
                    modifier = Modifier.fillMaxWidth(),
                    enabled = !isScanning,
                ) {
                    Icon(Icons.Filled.Refresh, contentDescription = null, modifier = Modifier.size(ButtonDefaults.IconSize))
                    Spacer(Modifier.size(ButtonDefaults.IconSpacing))
                    Text(if (isScanning) "Scanning…" else "Scan / Refresh")
                }
                if (isScanning) {
                    LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
                }
            }
            if (uiState.discoveredSessions.isEmpty()) {
                item {
                    Text("No sessions found. Try scanning again.")
                }
            }
            items(uiState.discoveredSessions, key = { it.id }) { session ->
                Card(modifier = Modifier.fillMaxWidth()) {
                    Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text(session.name, style = MaterialTheme.typography.titleMedium)
                        Text("Host: ${session.hostDeviceName}")
                        Text("Approval: ${session.approvalMode.label()}")
                        Text(
                            if (session.inviteCodeRequired) "Invite code required" else "Open — no code required",
                        )
                        Button(onClick = { onSelectSession(session) }) {
                            Text("Join")
                        }
                    }
                }
            }
        }
    }
}
