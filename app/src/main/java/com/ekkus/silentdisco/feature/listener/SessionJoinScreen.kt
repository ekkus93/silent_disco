package com.ekkus.silentdisco.feature.listener

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.relocation.BringIntoViewRequester
import androidx.compose.foundation.relocation.bringIntoViewRequester
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.ui.theme.WorkflowTouchTarget
import com.ekkus.silentdisco.app.JoinUiStep
import com.ekkus.silentdisco.app.UserProblemAction
import com.ekkus.silentdisco.app.derivedPersistentProblem
import com.ekkus.silentdisco.app.joinUiStep
import com.ekkus.silentdisco.app.label
import com.ekkus.silentdisco.core.model.ListenerLifecycleState

@OptIn(ExperimentalFoundationApi::class, ExperimentalMaterial3Api::class)
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
    val inviteCodeBringIntoView = remember { BringIntoViewRequester() }
    val inviteCodeFocus = remember { FocusRequester() }
    LaunchedEffect(editingInviteCode) {
        if (editingInviteCode && session?.inviteCodeRequired == true) {
            inviteCodeFocus.requestFocus()
            inviteCodeBringIntoView.bringIntoView()
        }
    }
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
        ClientWorkflowStepper(stage = ClientWorkflowStage.CONNECT)
        // The six-step progress timeline can exceed a compact viewport.
        // Pin the current authoritative stage or recovery problem above the
        // independently pinned actions, without requiring a scroll.
        val visibleProblem = problem.takeUnless { editingInviteCode }
        val activeStatus = when {
            visibleProblem != null -> visibleProblem.title
            requestStarted && uiState.listenerState != ListenerLifecycleState.PLAYING ->
                joinProgressLabel(uiState.joinUiStep())
            else -> null
        }
        if (activeStatus != null) {
            Text(
                text = activeStatus,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 8.dp)
                    .semantics {
                        liveRegion = if (visibleProblem != null) {
                            LiveRegionMode.Assertive
                        } else {
                            LiveRegionMode.Polite
                        }
                    }
                    .testTag("session-join-active-status"),
                style = MaterialTheme.typography.titleMedium,
                color = if (visibleProblem != null) {
                    MaterialTheme.colorScheme.error
                } else {
                    MaterialTheme.colorScheme.onSurface
                },
            )
        }
        Column(
            modifier = Modifier
                .weight(1f)
                .verticalScroll(rememberScrollState())
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(
                session?.name ?: "No session selected",
                style = MaterialTheme.typography.headlineMedium,
            )
            Text(
                "Hosted by ${session?.displayHostName ?: "Unknown host"}",
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
                            .bringIntoViewRequester(inviteCodeBringIntoView)
                            .focusRequester(inviteCodeFocus)
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
            } else {
                JoinProgressCard(uiState.joinUiStep())
            }

            if (problem != null && !editingInviteCode) {
                Card(modifier = Modifier.testTag("session-join-problem")) {
                    Column(
                        modifier = Modifier.padding(16.dp),
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Text(problem.title, style = MaterialTheme.typography.titleMedium)
                        Text(problem.detail, style = MaterialTheme.typography.bodyMedium)
                    }
                }
            }
        }

        if (uiState.listenerState != ListenerLifecycleState.PLAYING) {
            HorizontalDivider()
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 8.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                if (problem != null && !editingInviteCode) {
                    problem.primaryAction?.let { action ->
                        Button(
                            onClick = { handleProblemAction(action) },
                            modifier = Modifier
                                .fillMaxWidth()
                                .heightIn(min = WorkflowTouchTarget.minimum)
                                .testTag("session-problem-primary"),
                        ) { Text(action.label()) }
                    }
                    problem.secondaryAction?.let { action ->
                        TextButton(
                            onClick = { handleProblemAction(action) },
                            modifier = Modifier
                                .fillMaxWidth()
                                .heightIn(min = WorkflowTouchTarget.minimum)
                                .testTag("session-problem-secondary"),
                        ) { Text(action.label()) }
                    }
                } else if (!requestStarted || editingInviteCode) {
                    Button(
                        onClick = {
                            editingInviteCode = false
                            onJoin()
                        },
                        enabled = session != null &&
                            (!session.inviteCodeRequired || uiState.connectionProgress.inviteCode.isNotBlank()),
                        modifier = Modifier
                            .fillMaxWidth()
                            .heightIn(min = WorkflowTouchTarget.minimum)
                            .testTag("session-request-join"),
                    ) {
                        Text(if (editingInviteCode) "Request again" else "Request to join")
                    }
                }
                if (problem?.primaryAction != UserProblemAction.RETURN_TO_SESSIONS) {
                    TextButton(
                        onClick = onCancel,
                        modifier = Modifier
                            .fillMaxWidth()
                            .heightIn(min = WorkflowTouchTarget.minimum)
                            .testTag("session-join-cancel"),
                    ) {
                        Text("Cancel")
                    }
                }
            }
        }
    }
}

private val joinProgressSteps = listOf(
    JoinUiStep.FINDING_HOST to "Finding the host",
    JoinUiStep.CONNECTING to "Connecting",
    JoinUiStep.REQUESTING_ACCESS to "Requesting access",
    JoinUiStep.WAITING_FOR_APPROVAL to "Waiting for host approval",
    JoinUiStep.WAITING_FOR_PLAYBACK to "Waiting for the host to start playback",
    JoinUiStep.SYNCING_AUDIO to "Getting audio in sync",
)

private fun joinProgressLabel(step: JoinUiStep): String =
    joinProgressSteps.firstOrNull { it.first == step }?.second ?: "Playback is ready"

@Composable
private fun JoinProgressCard(activeStep: JoinUiStep) {
    val steps = joinProgressSteps
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
    JoinUiStep.CONNECTING -> "Opening a connection to the host."
    JoinUiStep.REQUESTING_ACCESS -> "Sending your request to the host."
    JoinUiStep.WAITING_FOR_APPROVAL -> "The host has not responded yet."
    JoinUiStep.WAITING_FOR_PLAYBACK ->
        "The host approved this device. Audio starts when the host presses Play."
    JoinUiStep.SYNCING_AUDIO -> "Buffering audio and matching the host’s timing."
    JoinUiStep.COMPLETE -> "Playback is ready."
}
