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
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.label
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.HostLifecycleState

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
            if (inviteCodeRequired) {
                OutlinedTextField(
                    value = uiState.hostForm.inviteCode,
                    onValueChange = onInviteCodeChanged,
                    modifier = Modifier
                        .fillMaxWidth()
                        .testTag("host-invite-code"),
                    label = { Text("Invite code") },
                    supportingText = { Text("Share this exact code with listeners.") },
                    singleLine = true,
                    isError = uiState.hostForm.inviteCode.isBlank(),
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
                (inviteCodeRequired && uiState.hostForm.inviteCode.isBlank())
            val isStarting = uiState.hostState == HostLifecycleState.CREATING_SESSION
            if (invalid) {
                Text(
                    text = "Complete the required session details before starting.",
                    color = MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.bodySmall,
                )
            }
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
