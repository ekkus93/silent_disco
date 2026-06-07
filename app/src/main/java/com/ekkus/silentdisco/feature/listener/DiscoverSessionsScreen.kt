package com.ekkus.silentdisco.feature.listener

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
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
import com.ekkus.silentdisco.app.label
import com.ekkus.silentdisco.core.model.SessionInfo

@Composable
fun DiscoverSessionsScreen(
    uiState: AppUiState,
    onRefresh: () -> Unit,
    onSelectSession: (SessionInfo) -> Unit,
) {
    LazyColumn(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        item {
            Text("Nearby Sessions", style = MaterialTheme.typography.headlineMedium)
        }
        item {
            Button(onClick = onRefresh, modifier = Modifier.fillMaxWidth()) {
                Text("Scan / Refresh")
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
