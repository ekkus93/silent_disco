package com.ekkus.silentdisco.feature.home

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.UserProblemAction
import com.ekkus.silentdisco.app.derivedPersistentProblem
import com.ekkus.silentdisco.app.persistentFeaturesEnabled
import com.ekkus.silentdisco.ui.components.AttentionBanner
import com.ekkus.silentdisco.ui.components.RoleActionCard

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RoleFirstHomeScreen(
    uiState: AppUiState,
    onHostClick: () -> Unit,
    onJoinClick: () -> Unit,
    onSettingsClick: () -> Unit,
    onRetryStorage: () -> Unit,
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
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(
                text = "What would you like to do?",
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
        }
    }
}
