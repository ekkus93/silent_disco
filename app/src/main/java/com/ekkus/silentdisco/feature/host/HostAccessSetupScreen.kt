package com.ekkus.silentdisco.feature.host

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
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
import com.ekkus.silentdisco.app.isValidInviteCode
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.ui.components.PrimaryProblemCard

private data class AccessOption(
    val mode: ApprovalMode,
    val title: String,
    val detail: String,
)

private val accessOptions = listOf(
    AccessOption(
        ApprovalMode.MANUAL,
        "Ask me before anyone joins",
        "You approve or reject every request.",
    ),
    AccessOption(
        ApprovalMode.INVITE_CODE,
        "Require an invite code",
        "Only people with the code can request access.",
    ),
    AccessOption(
        ApprovalMode.TRUSTED_DEVICES_PLACEHOLDER,
        "Approved devices only",
        "Devices you previously marked as always allowed can join.",
    ),
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HostAccessSetupScreen(
    uiState: AppUiState,
    onBack: () -> Unit,
    onApprovalModeChanged: (ApprovalMode) -> Unit,
    onInviteCodeChanged: (String) -> Unit,
    onGenerateCode: () -> Unit,
    onStartSession: () -> Unit,
    onOpenSettings: () -> Unit,
    onShareSupportReport: () -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .testTag("host-access-screen"),
    ) {
        TopAppBar(
            title = { Text("Choose access") },
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
            Text(
                text = "Who can join this session?",
                style = MaterialTheme.typography.headlineSmall,
            )

            accessOptions.forEach { option ->
                val selected = uiState.hostForm.approvalMode == option.mode
                Card(
                    modifier = Modifier
                        .fillMaxWidth()
                        .semantics {
                            contentDescription = buildString {
                                append(option.title)
                                append(". ")
                                append(option.detail)
                                if (selected) append(" Selected.")
                            }
                        }
                        .clickable(
                            role = Role.RadioButton,
                            onClick = { onApprovalModeChanged(option.mode) },
                        )
                        .testTag("host-access-${option.mode.name.lowercase()}"),
                ) {
                    Row(
                        modifier = Modifier.padding(16.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        RadioButton(selected = selected, onClick = null)
                        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            Text(option.title, style = MaterialTheme.typography.titleMedium)
                            Text(
                                option.detail,
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
            }

            val inviteCodeRequired = uiState.hostForm.approvalMode == ApprovalMode.INVITE_CODE
            val inviteCodeValid = !inviteCodeRequired || isValidInviteCode(uiState.hostForm.inviteCode)
            if (inviteCodeRequired) {
                OutlinedTextField(
                    value = uiState.hostForm.inviteCode,
                    onValueChange = onInviteCodeChanged,
                    modifier = Modifier
                        .fillMaxWidth()
                        .testTag("host-invite-code"),
                    label = { Text("Invite code") },
                    supportingText = {
                        Text(
                            if (uiState.hostForm.inviteCode.isBlank()) {
                                "Enter a 4-digit code or generate one."
                            } else if (!inviteCodeValid) {
                                "Invite codes must contain exactly 4 digits."
                            } else {
                                "Share this exact code with listeners."
                            },
                        )
                    },
                    singleLine = true,
                    isError = !inviteCodeValid,
                )
                TextButton(
                    onClick = onGenerateCode,
                    modifier = Modifier.testTag("host-generate-code"),
                ) {
                    Text("Generate code")
                }
            }

            Card(modifier = Modifier.fillMaxWidth()) {
                Column(
                    modifier = Modifier.padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    Text("Session summary", style = MaterialTheme.typography.titleMedium)
                    Text(uiState.hostForm.sessionName.ifBlank { "Unnamed session" })
                    Text(uiState.hostForm.selectedAudio?.displayName ?: "No audio selected")
                    Text(accessOptions.first { it.mode == uiState.hostForm.approvalMode }.title)
                    if (inviteCodeRequired) {
                        Text("Invite code: ${uiState.hostForm.inviteCode.ifBlank { "Required" }}")
                    }
                }
            }

            val invalid = uiState.hostForm.sessionName.isBlank() ||
                uiState.hostForm.selectedAudio == null ||
                !inviteCodeValid
            val isStarting = uiState.hostState == HostLifecycleState.CREATING_SESSION
            val startFailed = uiState.hostState == HostLifecycleState.ERROR && !uiState.lastError.isNullOrBlank()
            if (invalid) {
                Text(
                    text = "Complete the required session details before starting.",
                    color = MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.bodySmall,
                )
            }

            if (startFailed) {
                val permissionFailure = uiState.lastError.contains("permission", ignoreCase = true)
                PrimaryProblemCard(
                    title = if (permissionFailure) {
                        "Nearby-device access is required"
                    } else {
                        "The session could not be started"
                    },
                    detail = if (permissionFailure) {
                        "Allow nearby-device access in Settings, then start the session again."
                    } else {
                        "Check that nearby connections are available and try again."
                    },
                    primaryActionLabel = if (permissionFailure) "Open Settings" else "Retry",
                    onPrimaryAction = if (permissionFailure) onOpenSettings else onStartSession,
                    secondaryActionLabel = if (permissionFailure) "Retry" else "Share support report",
                    onSecondaryAction = if (permissionFailure) onStartSession else onShareSupportReport,
                    modifier = Modifier.testTag("host-start-problem"),
                )
            } else {
                Button(
                    onClick = onStartSession,
                    enabled = !invalid && !isStarting,
                    modifier = Modifier
                        .fillMaxWidth()
                        .testTag("host-start-session"),
                ) {
                    Text(if (isStarting) "Starting…" else "Start session")
                }
            }
        }
    }
}
