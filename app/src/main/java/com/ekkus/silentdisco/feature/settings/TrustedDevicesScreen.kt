package com.ekkus.silentdisco.feature.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
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
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.core.rust.RustTrustedDevice
import com.ekkus.silentdisco.ui.components.ConfirmationSheet
import com.ekkus.silentdisco.ui.components.EmptyState
import com.ekkus.silentdisco.ui.components.LoadingState
import com.ekkus.silentdisco.ui.components.PrimaryProblemCard
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TrustedDevicesScreen(
    uiState: TrustedDevicesUiState,
    onBack: () -> Unit,
    onRefresh: () -> Unit,
    onDelete: (String) -> Unit,
) {
    var pendingDelete by remember { mutableStateOf<RustTrustedDevice?>(null) }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .testTag("trusted-devices-screen"),
    ) {
        TopAppBar(
            title = { Text("Approved devices") },
            navigationIcon = {
                IconButton(onClick = onBack) {
                    Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                }
            },
        )

        when {
            uiState.isLoading && uiState.devices.isEmpty() -> LoadingState(
                title = "Loading approved devices",
                detail = "Reading the authoritative device list from local app data.",
                modifier = Modifier.testTag("trusted-devices-loading"),
            )

            uiState.error != null && uiState.devices.isEmpty() -> PrimaryProblemCard(
                title = "Approved devices are unavailable",
                detail = uiState.error,
                primaryActionLabel = "Try again",
                onPrimaryAction = onRefresh,
                modifier = Modifier
                    .padding(16.dp)
                    .testTag("trusted-devices-error"),
                primaryActionModifier = Modifier.testTag("trusted-devices-retry"),
            )

            uiState.devices.isEmpty() -> EmptyState(
                title = "No approved devices",
                detail = "Phones approved with Always allow will appear here.",
                actionLabel = "Refresh",
                onAction = onRefresh,
                modifier = Modifier.testTag("trusted-devices-empty"),
            )

            else -> LazyColumn(
                modifier = Modifier.fillMaxSize(),
                contentPadding = PaddingValues(16.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                item {
                    Text(
                        "These phones can join future sessions without asking again. Removing one does not disconnect an active listener.",
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }

                uiState.message?.let { message ->
                    item {
                        Card(
                            modifier = Modifier
                                .fillMaxWidth()
                                .testTag("trusted-devices-message"),
                        ) {
                            Text(
                                message,
                                modifier = Modifier.padding(16.dp),
                            )
                        }
                    }
                }

                uiState.error?.let { error ->
                    item {
                        PrimaryProblemCard(
                            title = "The last approved-device operation failed",
                            detail = error,
                            primaryActionLabel = "Refresh list",
                            onPrimaryAction = onRefresh,
                            modifier = Modifier.testTag("trusted-devices-inline-error"),
                        )
                    }
                }

                items(uiState.devices, key = RustTrustedDevice::deviceId) { device ->
                    TrustedDeviceCard(
                        device = device,
                        deleting = uiState.deletingDeviceId == device.deviceId,
                        actionsEnabled = uiState.deletingDeviceId == null && !uiState.isLoading,
                        onRemove = { pendingDelete = device },
                    )
                }
            }
        }
    }

    val device = pendingDelete
    ConfirmationSheet(
        visible = device != null,
        title = "Remove this approved device?",
        detail = device?.let {
            "${it.displayName} will need approval before joining a future session."
        }.orEmpty(),
        safeActionLabel = "Keep approved",
        destructiveActionLabel = "Remove approval",
        onDismiss = { pendingDelete = null },
        onConfirm = {
            val deviceId = device?.deviceId ?: return@ConfirmationSheet
            pendingDelete = null
            onDelete(deviceId)
        },
        testTag = "remove-trusted-device-confirmation",
    )
}

@Composable
private fun TrustedDeviceCard(
    device: RustTrustedDevice,
    deleting: Boolean,
    actionsEnabled: Boolean,
    onRemove: () -> Unit,
) {
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .testTag("trusted-device-${device.deviceId}"),
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                device.displayName,
                modifier = Modifier.semantics { heading() },
                style = MaterialTheme.typography.titleMedium,
            )
            Text(
                "Device ID: ${device.deviceId}",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text(
                trustedDeviceLastSeenLabel(device.lastSeenMs),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.End,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                if (deleting) {
                    CircularProgressIndicator(
                        modifier = Modifier
                            .size(24.dp)
                            .testTag("trusted-device-deleting-${device.deviceId}"),
                    )
                    Text(
                        "Removing approval…",
                        modifier = Modifier.padding(start = 8.dp),
                    )
                } else {
                    TextButton(
                        onClick = onRemove,
                        enabled = actionsEnabled,
                        modifier = Modifier.testTag("trusted-device-remove-${device.deviceId}"),
                    ) {
                        Text(
                            "Remove approval",
                            color = MaterialTheme.colorScheme.error,
                        )
                    }
                }
            }
        }
    }
}

internal fun trustedDeviceLastSeenLabel(lastSeenMs: Long): String {
    require(lastSeenMs >= 0L) { "Last-seen time must not be negative" }
    val formatted = DateTimeFormatter
        .ofLocalizedDateTime(FormatStyle.MEDIUM, FormatStyle.SHORT)
        .format(Instant.ofEpochMilli(lastSeenMs).atZone(ZoneId.systemDefault()))
    return "Last seen $formatted"
}
