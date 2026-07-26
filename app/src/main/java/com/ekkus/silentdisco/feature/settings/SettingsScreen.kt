package com.ekkus.silentdisco.feature.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
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
import com.ekkus.silentdisco.BuildConfig
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.StorageInitializationState
import com.ekkus.silentdisco.app.permissionSummary
import com.ekkus.silentdisco.ui.components.StatusBadge
import com.ekkus.silentdisco.ui.components.StatusTone

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(
    uiState: AppUiState,
    trustedDeviceManagementAvailable: Boolean,
    onBack: () -> Unit,
    onOpenSystemSettings: () -> Unit,
    onOpenTrustedDevices: () -> Unit,
    onOpenAdvancedDiagnostics: () -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .testTag("settings-screen"),
    ) {
        TopAppBar(
            title = { Text("Settings") },
            navigationIcon = {
                IconButton(onClick = onBack) {
                    Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
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
            SettingsCard(
                title = "Nearby-device permissions",
                modifier = Modifier.testTag("settings-permissions"),
            ) {
                Text(uiState.permissionSummary())
                StatusBadge(
                    text = if (uiState.permissions.isNotEmpty() && uiState.permissions.all { it.granted }) {
                        "Ready"
                    } else {
                        "Needs attention"
                    },
                    tone = if (uiState.permissions.isNotEmpty() && uiState.permissions.all { it.granted }) {
                        StatusTone.POSITIVE
                    } else {
                        StatusTone.ATTENTION
                    },
                    semanticLabel = "Nearby-device permissions: ${uiState.permissionSummary()}",
                )
                TextButton(onClick = onOpenSystemSettings) {
                    Text("Open system settings")
                }
            }

            SettingsCard(
                title = "Local app data",
                modifier = Modifier.testTag("settings-storage"),
            ) {
                val storageLabel = settingsStorageLabel(uiState.storageState)
                StatusBadge(
                    text = storageLabel,
                    tone = settingsStorageTone(uiState.storageState),
                    semanticLabel = "Local app data: $storageLabel",
                )
                uiState.storageError?.let {
                    Text(
                        "Additional troubleshooting information is available in Advanced diagnostics.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }

            if (trustedDeviceManagementAvailable) {
                SettingsCard(
                    title = "Approved devices",
                    modifier = Modifier.testTag("settings-approved-devices"),
                ) {
                    Text("Review phones that are always allowed to join your sessions.")
                    TextButton(onClick = onOpenTrustedDevices) {
                        Text("Manage approved devices")
                    }
                }
            }

            SettingsCard(
                title = "Troubleshooting",
                modifier = Modifier.testTag("settings-troubleshooting"),
            ) {
                Text("View technical connection, synchronization, and playback information.")
                TextButton(onClick = onOpenAdvancedDiagnostics) {
                    Text("Advanced diagnostics")
                }
            }

            SettingsCard(
                title = "About",
                modifier = Modifier.testTag("settings-about"),
            ) {
                Text("Silent Disco")
                Text("Version ${BuildConfig.VERSION_NAME}")
                Text("Build ${BuildConfig.VERSION_CODE}")
            }
        }
    }
}

@Composable
private fun SettingsCard(
    title: String,
    modifier: Modifier = Modifier,
    content: @Composable ColumnScope.() -> Unit,
) {
    Card(modifier = modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                title,
                modifier = Modifier.semantics { heading() },
                style = MaterialTheme.typography.titleMedium,
            )
            content()
        }
    }
}

internal fun settingsStorageLabel(state: StorageInitializationState): String = when (state) {
    StorageInitializationState.INITIALIZING -> "Opening local app data"
    StorageInitializationState.READY -> "Available"
    StorageInitializationState.RECOVERABLE_FAILURE -> "Temporarily unavailable"
    StorageInitializationState.FATAL_FAILURE -> "Could not be opened"
}

internal fun settingsStorageTone(state: StorageInitializationState): StatusTone = when (state) {
    StorageInitializationState.INITIALIZING -> StatusTone.IN_PROGRESS
    StorageInitializationState.READY -> StatusTone.POSITIVE
    StorageInitializationState.RECOVERABLE_FAILURE -> StatusTone.ATTENTION
    StorageInitializationState.FATAL_FAILURE -> StatusTone.CRITICAL
}
