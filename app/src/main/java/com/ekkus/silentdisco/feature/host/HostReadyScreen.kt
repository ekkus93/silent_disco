package com.ekkus.silentdisco.feature.host

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
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.isValidInviteCode
import com.ekkus.silentdisco.app.label
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.ui.theme.WorkflowSpacing
import com.ekkus.silentdisco.ui.theme.WorkflowTouchTarget

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HostReadyScreen(
    uiState: AppUiState,
    onBack: () -> Unit,
    onStartSession: () -> Unit,
    onOpenSettings: () -> Unit,
    onShareSupportReport: () -> Unit,
) {
    var startInFlight by rememberSaveable { mutableStateOf(false) }
    LaunchedEffect(uiState.hostState, uiState.lastError) {
        if (
            uiState.hostState == HostLifecycleState.ERROR ||
            uiState.hostState !in setOf(HostLifecycleState.IDLE, HostLifecycleState.CREATING_SESSION)
        ) {
            startInFlight = false
        }
    }
    val form = uiState.hostForm
    val files = listOfNotNull(form.selectedAudio) + form.additionalAudio
    val inviteCodeRequired = form.approvalMode == com.ekkus.silentdisco.core.model.ApprovalMode.INVITE_CODE
    val valid = form.sessionName.isNotBlank() && files.isNotEmpty() &&
        (!inviteCodeRequired || isValidInviteCode(form.inviteCode))
    val isStarting = uiState.hostState == HostLifecycleState.CREATING_SESSION || startInFlight
    val failed = uiState.hostState == HostLifecycleState.ERROR && !uiState.lastError.isNullOrBlank()

    fun requestStart() {
        if (!startInFlight && valid) {
            startInFlight = true
            onStartSession()
        }
    }

    Scaffold(
        modifier = Modifier.fillMaxSize().testTag("host-ready-screen"),
        topBar = {
            TopAppBar(
                title = { Text("Ready to host") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        },
        bottomBar = {
            Button(
                onClick = if (failed && uiState.lastError.orEmpty().contains("permission", ignoreCase = true)) {
                    onOpenSettings
                } else {
                    ::requestStart
                },
                enabled = (valid && !isStarting) || failed,
                modifier = Modifier.fillMaxWidth()
                    .padding(horizontal = WorkflowSpacing.large, vertical = WorkflowSpacing.medium)
                    .heightIn(min = WorkflowTouchTarget.minimum)
                    .testTag("host-start-session"),
            ) {
                Text(
                    when {
                        isStarting -> "Starting…"
                        failed && uiState.lastError.orEmpty().contains("permission", ignoreCase = true) -> "Open Settings"
                        failed -> "Retry"
                        else -> "Start hosting"
                    },
                )
            }
        },
    ) { insets ->
        Column(
            modifier = Modifier.fillMaxSize().padding(insets).verticalScroll(rememberScrollState())
                .padding(WorkflowSpacing.large),
            verticalArrangement = Arrangement.spacedBy(WorkflowSpacing.large),
        ) {
            Text("Check your session details before you start.", style = MaterialTheme.typography.bodyLarge)
            SummaryCard("Session", form.sessionName.ifBlank { "Add a session name" })
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(
                    modifier = Modifier.padding(WorkflowSpacing.large),
                    verticalArrangement = Arrangement.spacedBy(WorkflowSpacing.small),
                ) {
                    Text("Music (${files.size})", style = MaterialTheme.typography.titleMedium)
                    if (files.isEmpty()) {
                        Text("No songs selected", color = MaterialTheme.colorScheme.onSurfaceVariant)
                    } else {
                        files.forEachIndexed { index, file ->
                            Text("${index + 1}. ${file.displayName}")
                        }
                    }
                }
            }
            SummaryCard("Access", form.approvalMode.label())
            if (inviteCodeRequired) {
                SummaryCard("Invite code", form.inviteCode.ifBlank { "Required" })
            }
            if (failed) {
                val permissionFailure = uiState.lastError.orEmpty().contains("permission", ignoreCase = true)
                Card(modifier = Modifier.fillMaxWidth().testTag("host-start-problem")) {
                    Column(modifier = Modifier.padding(WorkflowSpacing.large), verticalArrangement = Arrangement.spacedBy(WorkflowSpacing.small)) {
                        Text(
                            if (permissionFailure) "Nearby-device access is required"
                            else "The session could not be started",
                            style = MaterialTheme.typography.titleMedium,
                        )
                        Text(
                            if (permissionFailure) {
                                "Allow nearby-device access in Settings, then try again."
                            } else {
                                "Check that nearby connections are available and try again."
                            },
                        )
                        TextButton(
                            onClick = if (permissionFailure) ::requestStart else onShareSupportReport,
                            modifier = Modifier.heightIn(min = WorkflowTouchTarget.minimum),
                        ) {
                            Text(if (permissionFailure) "Retry" else "Share support report")
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun SummaryCard(title: String, detail: String) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(WorkflowSpacing.large),
            horizontalArrangement = Arrangement.spacedBy(WorkflowSpacing.large),
        ) {
            Text(title, style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
            Text(detail, modifier = Modifier.weight(2f))
        }
    }
}
