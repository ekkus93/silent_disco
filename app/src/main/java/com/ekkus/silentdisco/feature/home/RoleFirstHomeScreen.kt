package com.ekkus.silentdisco.feature.home

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.PermissionRequestContext
import com.ekkus.silentdisco.app.RecentAvailability
import com.ekkus.silentdisco.app.UserProblemAction
import com.ekkus.silentdisco.app.derivedPersistentProblem
import com.ekkus.silentdisco.app.missingPermissions
import com.ekkus.silentdisco.app.persistentFeaturesEnabled
import com.ekkus.silentdisco.core.model.AppRole
import com.ekkus.silentdisco.core.rust.P2RecentSession
import com.ekkus.silentdisco.ui.components.AttentionBanner
import com.ekkus.silentdisco.ui.components.RoleActionCard
import java.text.DateFormat
import java.util.Date

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RoleFirstHomeScreen(
    uiState: AppUiState,
    onHostClick: () -> Unit,
    onJoinClick: () -> Unit,
    onSettingsClick: () -> Unit,
    onRetryStorage: () -> Unit,
    recentSession: P2RecentSession? = null,
    recentAvailability: RecentAvailability = RecentAvailability.IDLE,
    onCheckRecentSession: (() -> Unit)? = null,
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .testTag("home-screen"),
    ) {
        TopAppBar(
            title = { Text("Silent Disco") },
            actions = {
                IconButton(
                    onClick = onSettingsClick,
                    modifier = Modifier.testTag("home-settings"),
                ) {
                    Icon(Icons.Filled.Settings, contentDescription = "Settings")
                }
            },
        )
        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(
                text = "What would you like to do?",
                modifier = Modifier.semantics { heading() },
                style = MaterialTheme.typography.headlineMedium,
            )
            Text(
                text = "Play and listen with nearby phones — no internet required.",
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            uiState.derivedPersistentProblem()?.let { problem ->
                AttentionBanner(
                    title = problem.title,
                    detail = problem.detail,
                    actionLabel = if (problem.primaryAction == UserProblemAction.RETRY) "Retry" else null,
                    onAction = if (problem.primaryAction == UserProblemAction.RETRY) onRetryStorage else null,
                    modifier = Modifier.testTag("home-attention"),
                )
            }

            val deniedContext = when (uiState.selectedRole) {
                AppRole.HOST -> PermissionRequestContext.HOST_NEARBY
                AppRole.LISTENER -> PermissionRequestContext.LISTENER_NEARBY
                null -> null
            }?.takeIf { context ->
                uiState.permissions.isNotEmpty() && uiState.missingPermissions(context).isNotEmpty()
            }
            if (deniedContext != null) {
                AttentionBanner(
                    title = "Nearby device access is required",
                    detail = deniedContext.explanation,
                    actionLabel = "Open Settings",
                    onAction = onSettingsClick,
                    modifier = Modifier.testTag("home-permission-attention"),
                )
            }

            val enabled = uiState.persistentFeaturesEnabled()
            RoleActionCard(
                title = "Host music",
                description = "Play music for nearby listeners.",
                actionLabel = "Start a session",
                enabled = enabled,
                onClick = onHostClick,
                modifier = Modifier.testTag("home-host"),
            )
            RoleActionCard(
                title = "Join music",
                description = "Listen to a nearby host in sync.",
                actionLabel = "Find a session",
                enabled = enabled,
                onClick = onJoinClick,
                modifier = Modifier.testTag("home-join"),
            )

            if (recentSession != null && onCheckRecentSession != null) {
                Card(
                    modifier = Modifier
                        .fillMaxWidth()
                        .testTag("home-recent-session"),
                ) {
                    Column(
                        modifier = Modifier.padding(16.dp),
                        verticalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        Text("Recent session", style = MaterialTheme.typography.titleMedium)
                        Text(recentSession.sessionName, style = MaterialTheme.typography.titleLarge)
                        Text("Hosted by ${recentSession.hostName}")
                        Text(
                            "Last used ${DateFormat.getDateTimeInstance().format(Date(recentSession.endedAtMs))}",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Text(
                            when (recentAvailability) {
                                RecentAvailability.IDLE -> "Availability has not been checked."
                                RecentAvailability.CHECKING -> "Checking nearby discovery…"
                                RecentAvailability.AVAILABLE -> "This exact session is nearby."
                                RecentAvailability.UNAVAILABLE -> "This session is not currently nearby."
                            },
                            modifier = Modifier.testTag("home-recent-availability"),
                        )
                        TextButton(
                            onClick = onCheckRecentSession,
                            enabled = enabled && recentAvailability != RecentAvailability.CHECKING,
                            modifier = Modifier.testTag("home-check-recent"),
                        ) {
                            Text(
                                if (recentAvailability == RecentAvailability.AVAILABLE) {
                                    "Open session"
                                } else {
                                    "Check availability"
                                },
                            )
                        }
                    }
                }
            }
        }
    }
}
