package com.ekkus.silentdisco.feature.listener

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.canSelectSession
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.SessionInfo

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NearbySessionsScreen(
    uiState: AppUiState,
    permissionRequired: Boolean,
    onBack: () -> Unit,
    onRequestPermission: () -> Unit,
    onRefresh: () -> Unit,
    onSelectSession: (SessionInfo) -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .testTag("nearby-sessions-screen"),
    ) {
        TopAppBar(
            title = { Text("Nearby sessions") },
            navigationIcon = {
                IconButton(onClick = onBack) {
                    Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                }
            },
            actions = {
                IconButton(
                    onClick = onRefresh,
                    enabled = !uiState.isScanning && !permissionRequired,
                ) {
                    Icon(Icons.Filled.Refresh, contentDescription = "Refresh nearby sessions")
                }
            },
        )

        when {
            permissionRequired -> PermissionRequiredState(onRequestPermission)
            uiState.isScanning && uiState.discoveredSessions.isEmpty() -> ScanningState()
            uiState.discoveredSessions.isEmpty() && !uiState.lastError.isNullOrBlank() -> ScanFailureState(
                detail = uiState.lastError,
                onRetry = onRefresh,
            )
            uiState.discoveredSessions.isEmpty() -> EmptySessionsState(onRefresh)
            else -> SessionResults(
                uiState = uiState,
                onRefresh = onRefresh,
                onSelectSession = onSelectSession,
            )
        }
    }
}

@Composable
private fun PermissionRequiredState(onRequestPermission: () -> Unit) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(24.dp)
            .testTag("nearby-permission-required"),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text("Allow nearby device access", style = MaterialTheme.typography.headlineSmall)
        Text(
            "This lets your phone find Silent Disco sessions near you.",
            modifier = Modifier.padding(vertical = 12.dp),
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Button(onClick = onRequestPermission) { Text("Continue") }
    }
}

@Composable
private fun ScanningState() {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(24.dp)
            .testTag("nearby-scanning"),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        CircularProgressIndicator()
        Text(
            "Looking for nearby sessions…",
            modifier = Modifier.padding(top = 16.dp),
            style = MaterialTheme.typography.titleMedium,
        )
        Text(
            "Sessions will appear as nearby hosts become available.",
            modifier = Modifier.padding(top = 8.dp),
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun ScanFailureState(detail: String, onRetry: () -> Unit) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(24.dp)
            .testTag("nearby-scan-failure"),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text("Couldn’t look for sessions", style = MaterialTheme.typography.headlineSmall)
        Text(
            detail,
            modifier = Modifier.padding(vertical = 12.dp),
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Button(onClick = onRetry) { Text("Retry") }
    }
}

@Composable
private fun EmptySessionsState(onRefresh: () -> Unit) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(24.dp)
            .testTag("nearby-empty"),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text("No nearby sessions found", style = MaterialTheme.typography.headlineSmall)
        Text(
            "Ask the host to start their session, then try again.",
            modifier = Modifier.padding(vertical = 12.dp),
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Button(onClick = onRefresh) { Text("Look again") }
    }
}

@Composable
private fun SessionResults(
    uiState: AppUiState,
    onRefresh: () -> Unit,
    onSelectSession: (SessionInfo) -> Unit,
) {
    LazyColumn(
        modifier = Modifier.testTag("nearby-results"),
        contentPadding = PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column {
                    Text("Nearby now", style = MaterialTheme.typography.titleLarge)
                    Text(
                        "Results may change as hosts appear or leave.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                if (uiState.isScanning) CircularProgressIndicator()
                else TextButton(onClick = onRefresh) { Text("Refresh") }
            }
        }
        items(
            items = uiState.discoveredSessions.sortedWith(
                compareBy(String.CASE_INSENSITIVE_ORDER) { it.name },
            ),
            key = SessionInfo::id,
        ) { session ->
            NearbySessionCard(
                session = session,
                enabled = uiState.canSelectSession(session),
                onClick = { onSelectSession(session) },
            )
        }
    }
}

@Composable
private fun NearbySessionCard(
    session: SessionInfo,
    enabled: Boolean,
    onClick: () -> Unit,
) {
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick),
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(session.name, style = MaterialTheme.typography.titleLarge)
            Text(
                "Hosted by ${session.hostDeviceName}",
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text(
                when (session.approvalMode) {
                    ApprovalMode.MANUAL -> "Approval required"
                    ApprovalMode.INVITE_CODE -> "Invite code required"
                    ApprovalMode.TRUSTED_DEVICES_PLACEHOLDER -> "Approved devices only"
                },
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.primary,
            )
            Button(onClick = onClick, enabled = enabled) { Text("Join") }
            if (!enabled) {
                Text(
                    "Finish or cancel the current join attempt before choosing another session.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}
