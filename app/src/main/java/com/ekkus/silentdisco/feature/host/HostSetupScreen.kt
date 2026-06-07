package com.ekkus.silentdisco.feature.host

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.label
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.HostLifecycleState

@Composable
fun HostSetupScreen(
    uiState: AppUiState,
    onSessionNameChanged: (String) -> Unit,
    onApprovalModeChanged: (ApprovalMode) -> Unit,
    onInviteCodeChanged: (String) -> Unit,
    onRememberApprovedChanged: (Boolean) -> Unit,
    onPickAudio: () -> Unit,
    onStartHosting: () -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text("Create Host Session", style = MaterialTheme.typography.headlineMedium)
        OutlinedTextField(
            value = uiState.hostForm.sessionName,
            onValueChange = onSessionNameChanged,
            modifier = Modifier.fillMaxWidth(),
            label = { Text("Session name") },
        )
        Card(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text("Approval mode", style = MaterialTheme.typography.titleMedium)
                ApprovalMode.entries.forEach { mode ->
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        modifier = Modifier
                            .fillMaxWidth()
                            .selectable(
                                selected = uiState.hostForm.approvalMode == mode,
                                onClick = { onApprovalModeChanged(mode) },
                                role = Role.RadioButton,
                            ),
                    ) {
                        RadioButton(
                            selected = uiState.hostForm.approvalMode == mode,
                            onClick = null,
                        )
                        Text(mode.label(), modifier = Modifier.padding(start = 8.dp))
                    }
                }
                OutlinedTextField(
                    value = uiState.hostForm.inviteCode,
                    onValueChange = onInviteCodeChanged,
                    modifier = Modifier.fillMaxWidth(),
                    label = { Text("Optional invite code") },
                )
                if (uiState.hostForm.approvalMode == ApprovalMode.INVITE_CODE) {
                    Text(
                        text = "Listeners will enter code: ${uiState.hostForm.inviteCode.ifBlank { "Auto-generated when hosting" }}",
                        style = MaterialTheme.typography.bodyMedium,
                    )
                }
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier
                        .fillMaxWidth()
                        .toggleable(
                            value = uiState.hostForm.rememberApprovedDevices,
                            onValueChange = onRememberApprovedChanged,
                            role = Role.Checkbox,
                        ),
                ) {
                    Checkbox(
                        checked = uiState.hostForm.rememberApprovedDevices,
                        onCheckedChange = null,
                    )
                    Text("Remember approved devices", modifier = Modifier.padding(start = 8.dp))
                }
            }
        }
        Card(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Audio source", style = MaterialTheme.typography.titleMedium)
                Text(uiState.hostForm.selectedAudio?.displayName ?: "No audio file selected")
                Button(onClick = onPickAudio) {
                    Text("Choose Audio File")
                }
            }
        }
        val isStarting = uiState.hostState == HostLifecycleState.CREATING_SESSION
        Button(
            onClick = onStartHosting,
            modifier = Modifier.fillMaxWidth(),
            enabled = !isStarting,
        ) {
            Text(if (isStarting) "Starting…" else "Start Hosting")
        }
    }
}
