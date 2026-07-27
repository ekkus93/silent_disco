package com.ekkus.silentdisco.feature.p2

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.P2StorageState
import com.ekkus.silentdisco.app.P2UiState
import com.ekkus.silentdisco.ui.components.EmptyState
import com.ekkus.silentdisco.ui.components.LoadingState
import com.ekkus.silentdisco.ui.components.PrimaryProblemCard
import java.text.DateFormat
import java.util.Date

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TrustedHostsScreen(
    uiState: P2UiState,
    onBack: () -> Unit,
    onDelete: (String) -> Unit,
) {
    Scaffold(
        modifier = Modifier.testTag("trusted-hosts-screen"),
        topBar = {
            TopAppBar(
                title = { Text("Trusted hosts") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        },
    ) { padding ->
        when {
            uiState.storageState == P2StorageState.INITIALIZING -> LoadingState(
                title = "Loading trusted hosts…",
                detail = "Reading verified host keys from Rust-owned storage",
                modifier = Modifier
                    .fillMaxSize()
                    .padding(padding),
            )
            uiState.storageState == P2StorageState.ERROR -> PrimaryProblemCard(
                title = "Trusted hosts are unavailable",
                detail = uiState.lastError ?: "Optional P2 storage could not be opened.",
                primaryActionLabel = null,
                onPrimaryAction = null,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(padding)
                    .padding(16.dp),
            )
            uiState.trustedHosts.isEmpty() -> EmptyState(
                title = "No trusted hosts yet",
                detail = "Scan a host's signed QR invitation and choose Trust host and join. Display names alone never create trust.",
                modifier = Modifier
                    .fillMaxSize()
                    .padding(padding),
            )
            else -> LazyColumn(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(padding)
                    .testTag("trusted-hosts-list"),
                contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                item {
                    Text(
                        "A trusted host is identified by its verified public key, not by its visible name.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                items(uiState.trustedHosts, key = { it.fingerprint }) { host ->
                    Card(modifier = Modifier.fillMaxWidth()) {
                        Row(
                            modifier = Modifier.padding(16.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            Column(modifier = Modifier.weight(1f)) {
                                Text(host.displayName, style = MaterialTheme.typography.titleMedium)
                                Text(
                                    "Verified ${DateFormat.getDateTimeInstance().format(Date(host.lastVerifiedMs))}",
                                    style = MaterialTheme.typography.bodySmall,
                                )
                                Text(
                                    "Key ${host.fingerprint.take(16)}…",
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                            IconButton(
                                onClick = { onDelete(host.fingerprint) },
                                modifier = Modifier.testTag("delete-trusted-host-${host.fingerprint.take(8)}"),
                            ) {
                                Icon(Icons.Filled.Delete, contentDescription = "Remove trusted host")
                            }
                        }
                    }
                }
            }
        }
    }
}
