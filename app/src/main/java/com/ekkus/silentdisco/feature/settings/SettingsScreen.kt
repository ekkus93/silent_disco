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
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.BuildConfig
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.StorageInitializationState
import com.ekkus.silentdisco.app.permissionSummary

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
            SettingsCard("Nearby-device permissions") {
                Text(uiState.permissionSummary())
                TextButton(onClick = onOpenSystemSettings) {
                    Text("Open system settings")
                }
            }

            SettingsCard("Local app data") {
                Text(
                    when (uiState.storageState) {
                        StorageInitializationState.INITIALIZING -> "Opening local app data…"
                        StorageInitializationState.READY -> "Local app data is available"
                        StorageInitializationState.RECOVERABLE_FAILURE -> "Local app data is temporarily unavailable"
                        StorageInitializationState.FATAL_FAILURE -> "Local app data could not be opened"
                    },
                )
                uiState.storageError?.let {
                    Text(
                        it,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }

            if (trustedDeviceManagementAvailable) {
                SettingsCard("Approved devices") {
                    Text("Review phones that are always allowed to join your sessions.")
                    TextButton(onClick = onOpenTrustedDevices) {
                        Text("Manage approved devices")
                    }
                }
            }

            SettingsCard("Troubleshooting") {
                Text("View technical connection, synchronization, and playback information.")
                TextButton(onClick = onOpenAdvancedDiagnostics) {
                    Text("Advanced diagnostics")
                }
            }

            SettingsCard("About") {
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
    content: @Composable ColumnScope.() -> Unit,
) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(title, style = MaterialTheme.typography.titleMedium)
            content()
        }
    }
}
