package com.ekkus.silentdisco.feature.host

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
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
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.ui.theme.WorkflowSpacing
import com.ekkus.silentdisco.ui.theme.WorkflowTouchTarget
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
    onNext: () -> Unit,
) {
    var startSubmissionInFlight by rememberSaveable { mutableStateOf(false) }

    LaunchedEffect(uiState.hostState, uiState.lastError) {
        if (
            uiState.hostState == HostLifecycleState.ERROR ||
            uiState.hostState !in setOf(
                HostLifecycleState.IDLE,
                HostLifecycleState.CREATING_SESSION,
            )
        ) {
            startSubmissionInFlight = false
        }
    }

    fun requestNext() {
        if (startSubmissionInFlight) return
        startSubmissionInFlight = true
        onNext()
    }
    val inviteCodeRequired = uiState.hostForm.approvalMode == ApprovalMode.INVITE_CODE
    val inviteCodeValid = !inviteCodeRequired || isValidInviteCode(uiState.hostForm.inviteCode)
    val invalid = uiState.hostForm.sessionName.isBlank() ||
        uiState.hostForm.selectedAudio == null || !inviteCodeValid
    val isStarting = uiState.hostState == HostLifecycleState.CREATING_SESSION || startSubmissionInFlight

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
                .weight(1f)
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(WorkflowSpacing.large),
            verticalArrangement = Arrangement.spacedBy(WorkflowSpacing.large),
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
                        modifier = Modifier.padding(WorkflowSpacing.large),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(WorkflowSpacing.medium),
                    ) {
                        RadioButton(selected = selected, onClick = null)
                        Column(verticalArrangement = Arrangement.spacedBy(WorkflowSpacing.xSmall)) {
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
                    modifier = Modifier.heightIn(min = WorkflowTouchTarget.minimum)
                        .testTag("host-generate-code"),
                ) {
                    Text("Generate code")
                }
            }

            Card(modifier = Modifier.fillMaxWidth()) {
                Column(
                    modifier = Modifier.padding(WorkflowSpacing.large),
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

            if (invalid) {
                Text(
                    text = "Complete the required session details before continuing.",
                    color = MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.bodySmall,
                )
            }

        }
        Button(
            onClick = ::requestNext,
            enabled = !invalid && !isStarting,
                    modifier = Modifier.fillMaxWidth()
                        .padding(horizontal = WorkflowSpacing.large, vertical = WorkflowSpacing.medium)
                        .heightIn(min = WorkflowTouchTarget.minimum)
                .testTag("host-access-next"),
        ) {
            Text(if (isStarting) "Continuing…" else "Review session")
        }
    }
}
