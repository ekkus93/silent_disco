package com.ekkus.silentdisco.feature.listener

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
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.JoinUiStep
import com.ekkus.silentdisco.app.UserProblemAction
import com.ekkus.silentdisco.app.derivedPersistentProblem
import com.ekkus.silentdisco.app.joinUiStep
import com.ekkus.silentdisco.app.label
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.ui.components.PrimaryProblemCard

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SessionJoinScreen(
    uiState: AppUiState,
    onInviteCodeChanged: (String) -> Unit,
    onJoin: () -> Unit,
    onCancel: () -> Unit,
    onRetry: () -> Unit,
    onReturnToSessions: () -> Unit,
    onOpenSettings: () -> Unit,
    onShareSupportReport: () -> Unit,
) {
    val session = uiState.selectedSession
    val problem = uiState.derivedPersistentProblem()
    var editingInviteCode by rememberSaveable { mutableStateOf(false) }
    val requestStarted = !editingInviteCode && uiState.listenerState !in setOf(
        ListenerLifecycleState.IDLE,
        ListenerLifecycleState.SCANNING,
        ListenerLifecycleState.SESSION_SELECTED,
    )

    fun handleProblemAction(action: UserProblemAction) {
        when (action) {
            UserProblemAction.RETRY,
            UserProblemAction.RECONNECT,
            UserProblemAction.RESYNCHRONIZE -> onRetry()

            UserProblemAction.EDIT_CODE -> editingInviteCode = true
            UserProblemAction.RETURN_TO_SESSIONS -> onReturnToSessions()
            UserProblemAction.OPEN_SETTINGS -> onOpenSettings()
            UserProblemAction.SHARE_SUPPORT_REPORT -> onShareSupportReport()
            UserProblemAction.DISMISS -> Unit
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .testTag("session-join-screen"),
    ) {
        TopAppBar(
            title = { Text("Join session") },
            navigationIcon = {
                IconButton(onClick = onCancel) {
                    Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Cancel and go back")
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
                session?.name ?: "No session selected",
                style = MaterialTheme.typography.headlineMedium,
            )
            Text(
                "Hosted by ${session?.hostDeviceName ?: "Unknown host"}",
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            if (!requestStarted) {
                Text(
                    if (session?.inviteCodeRequired == true) {
                        "Enter the host’s invite code, then request access."
                    } else {
                        "The host may need to approve your request."
                    },
                    style = MaterialTheme.typography.bodyLarge,
                )
                if (session?.inviteCodeRequired == true) {
                    OutlinedTextField(
                        value = uiState.connectionProgress.inviteCode,
                        onValueChange = onInviteCodeChanged,
                        modifier = Modifier
                            .fillMaxWidth()
                            .testTag("session-join-code"),
                        label = { Text("Invite code") },
                        supportingText = if (editingInviteCode) {
                            { Text("Check the exact code with the host before trying again.") }
                        } else {
                            null
                        },
                        singleLine = true,
                    )
                }
                Button(
                    onClick = {
                        editingInviteCode = false
                        onJoin()
                    },
                    enabled = session != null &&
                        (!session.inviteCodeRequired || uiState.connectionProgress.inviteCode.isNotBlank()),
                    modifier = Modifier
                        .fillMaxWidth()
                        .testTag("session-request-join"),
                ) {
                    Text(if (editingInviteCode) "Request again" else "Request to join")
                }
            } else {
                JoinProgressCard(uiState.joinUiStep())
            }

            if (problem != null && !editingInviteCode) {
                PrimaryProblemCard(
                    title = problem.title,
                    detail = problem.detail,
                    primaryActionLabel = problem.primaryAction?.label(),
                    onPrimaryAction = problem.primaryAction?.let { action ->
                        { handleProblemAction(action) }
                    },
                    secondaryActionLabel = problem.secondaryAction?.label(),
                    onSecondaryAction = problem.secondaryAction?.let { action ->
                        { handleProblemAction(action) }
                    },
                    modifier = Modifier.testTag("session-join-problem"),
                    primaryActionModifier = Modifier.testTag("session-problem-primary"),
                    secondaryActionModifier = Modifier.testTag("session-problem-secondary"),
                )
            }

            if (uiState.listenerState != ListenerLifecycleState.PLAYING) {
                TextButton(onClick = onCancel, modifier = Modifier.fillMaxWidth()) {
                    Text("Cancel")
                }
            }
        }
    }
}

@Composable
private fun JoinProgressCard(activeStep: JoinUiStep) {
    val steps = listOf(
        JoinUiStep.FINDING_HOST to "Finding the host",
        JoinUiStep.REQUESTING_ACCESS to "Requesting access",
        JoinUiStep.WAITING_FOR_APPROVAL to "Waiting for host approval",
        JoinUiStep.CONNECTING to "Connecting",
        JoinUiStep.SYNCING_AUDIO to "Getting audio in sync",
    )
    val activeIndex = when (activeStep) {
        JoinUiStep.COMPLETE -> steps.lastIndex + 1
        else -> steps.indexOfFirst { it.first == activeStep }.coerceAtLeast(0)
    }

    Card(
        modifier = Modifier
            .fillMaxWidth()
            .testTag("session-join-progress"),
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            steps.forEachIndexed { index, (_, label) ->
                val complete = index < activeIndex
                val active = index == activeIndex
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    when {
                        complete -> Text(
                            "✓",
                            modifier = Modifier.semantics {
                                contentDescription = "$label complete"
                            },
                            color = MaterialTheme.colorScheme.primary,
                            style = MaterialTheme.typography.titleMedium,
                        )

                        active -> CircularProgressIndicator(
                            modifier = Modifier.semantics {
                                contentDescription = "$label in progress"
                            },
                        )

                        else -> Text(
                            "○",
                            modifier = Modifier.semantics {
                                contentDescription = "$label pending"
                            },
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            style = MaterialTheme.typography.titleMedium,
                        )
                    }
                    Column {
                        Text(
                            label,
                            style = if (active) {
                                MaterialTheme.typography.titleMedium
                            } else {
                                MaterialTheme.typography.bodyLarge
                            },
                            color = if (!complete && !active) {
                                MaterialTheme.colorScheme.onSurfaceVariant
                            } else {
                                MaterialTheme.colorScheme.onSurface
                            },
                        )
                        if (active) {
                            Text(
                                activeStepDetail(activeStep),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
            }
        }
    }
}

private fun activeStepDetail(step: JoinUiStep): String = when (step) {
    JoinUiStep.FINDING_HOST -> "Checking that the nearby host is still available."
    JoinUiStep.REQUESTING_ACCESS -> "Sending your request to the host."
    JoinUiStep.WAITING_FOR_APPROVAL -> "The host has not responded yet."
    JoinUiStep.CONNECTING -> "Creating the nearby connection."
    JoinUiStep.SYNCING_AUDIO -> "Buffering audio and matching the host’s timing."
    JoinUiStep.COMPLETE -> "Playback is ready."
}
