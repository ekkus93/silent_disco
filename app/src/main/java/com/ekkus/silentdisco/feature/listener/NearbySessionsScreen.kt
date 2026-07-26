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
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.canSelectSession
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.ui.components.EmptyState
import com.ekkus.silentdisco.ui.components.LoadingState
import com.ekkus.silentdisco.ui.components.PrimaryProblemCard
import com.ekkus.silentdisco.ui.components.StatusBadge
import com.ekkus.silentdisco.ui.components.StatusTone

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
            permissionRequired -> EmptyState(
                title = "Allow nearby device access",
                detail = "This lets your phone find Silent Disco sessions near you.",
                actionLabel = "Continue",
                onAction = onRequestPermission,
                modifier = Modifier.testTag("nearby-permission-required"),
            )

            uiState.isScanning && uiState.discoveredSessions.isEmpty() -> LoadingState(
                title = "Looking for nearby sessions",
                detail = "Sessions will appear as nearby hosts become available.",
                modifier = Modifier.testTag("nearby-scanning"),
            )

            uiState.discoveredSessions.isEmpty() && !uiState.lastError.isNullOrBlank() ->
                PrimaryProblemCard(
                    title = "Couldn’t look for sessions",
                    detail = "Check nearby-device access and try again.",
                    primaryActionLabel = "Retry",
                    onPrimaryAction = onRefresh,
                    modifier = Modifier
                        .padding(24.dp)
                        .testTag("nearby-scan-failure"),
                )

            uiState.discoveredSessions.isEmpty() -> EmptyState(
                title = "No nearby sessions found",
                detail = "Ask the host to start their session, then try again.",
                actionLabel = "Look again",
                onAction = onRefresh,
                modifier = Modifier.testTag("nearby-empty"),
            )

            else -> SessionResults(
                uiState = uiState,
                onRefresh = onRefresh,
                onSelectSession = onSelectSession,
            )
        }
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
                if (uiState.isScanning) {
                    CircularProgressIndicator(
                        modifier = Modifier.semantics {
                            contentDescription = "Refreshing nearby sessions"
                        },
                    )
                } else {
                    TextButton(onClick = onRefresh) { Text("Refresh") }
                }
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
    val badgeText = sessionAccessLabel(session)
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .semantics {
                contentDescription = buildString {
                    append(session.name)
                    append(", hosted by ")
                    append(session.hostDeviceName)
                    append(", ")
                    append(badgeText)
                    if (!enabled) append(", unavailable while another join is active")
                }
            }
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
            StatusBadge(
                text = badgeText,
                tone = when (session.approvalMode) {
                    ApprovalMode.MANUAL -> StatusTone.ATTENTION
                    ApprovalMode.INVITE_CODE -> StatusTone.NEUTRAL
                    ApprovalMode.TRUSTED_DEVICES_PLACEHOLDER -> StatusTone.POSITIVE
                },
            )
            Button(
                onClick = onClick,
                enabled = enabled,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text("Join")
            }
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

internal fun sessionAccessLabel(session: SessionInfo): String = when (session.approvalMode) {
    ApprovalMode.MANUAL -> "Approval required"
    ApprovalMode.INVITE_CODE -> "Invite code required"
    ApprovalMode.TRUSTED_DEVICES_PLACEHOLDER -> "Approved devices only"
}
