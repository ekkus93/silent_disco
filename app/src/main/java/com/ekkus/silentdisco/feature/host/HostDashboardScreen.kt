package com.ekkus.silentdisco.feature.host

import android.os.SystemClock
import android.os.Handler
import android.os.Looper
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.QueueMusic
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Add
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.PrimaryTabRow
import androidx.compose.material3.Tab
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.BuildConfig
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.ui.theme.WorkflowSpacing
import com.ekkus.silentdisco.ui.theme.WorkflowTouchTarget
import com.ekkus.silentdisco.app.JoinApprovalAction
import com.ekkus.silentdisco.app.SessionHealthLevel
import com.ekkus.silentdisco.app.canStartOrPauseHostPlayback
import com.ekkus.silentdisco.app.canStopHostPlayback
import com.ekkus.silentdisco.app.hostSessionHealthSummary
import com.ekkus.silentdisco.app.label
import com.ekkus.silentdisco.core.model.JoinRequest
import com.ekkus.silentdisco.core.model.ListenerInfo
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.model.TrustState

enum class HostDashboardTab(val title: String) {
    REQUESTS("Requests"),
    CONNECTED("Connected"),
    NEEDS_ATTENTION("Needs attention"),
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HostDashboardScreen(
    uiState: AppUiState,
    onBackRequest: () -> Unit,
    onInvite: () -> Unit,
    onApproval: (JoinRequest, JoinApprovalAction) -> Unit,
    onRemoveListener: (String) -> Unit,
    onPlayPause: () -> Unit,
    onStop: () -> Unit,
    onEndSessionRequest: () -> Unit,
    onOpenConnectionHelp: () -> Unit,
    onAddDemoJoinRequest: () -> Unit,
    onAddSongs: () -> Unit = {},
    onManageQueue: () -> Unit = {},
    initialTab: HostDashboardTab = HostDashboardTab.REQUESTS,
) {
    var selectedTabIndex by rememberSaveable(initialTab) { mutableIntStateOf(initialTab.ordinal) }
    var overflowOpen by remember { mutableStateOf(false) }
    var approvalInFlightRequestId by rememberSaveable { mutableStateOf<String?>(null) }
    var approvalInFlightAction by rememberSaveable { mutableStateOf<JoinApprovalAction?>(null) }
    var approvalBaselineSignature by rememberSaveable { mutableStateOf("") }
    val health = uiState.hostSessionHealthSummary()
    val troubledListeners = uiState.approvedListeners.filter(::listenerNeedsAttention)
    val currentProblemSignature = "${uiState.lastMessage.orEmpty()}|${uiState.lastError.orEmpty()}"

    LaunchedEffect(uiState.pendingJoinRequests, uiState.approvedListeners, currentProblemSignature) {
        val inFlight = approvalInFlightRequestId ?: return@LaunchedEffect
        val requestCompleted = uiState.pendingJoinRequests.none { it.requestId == inFlight }
        val operationReportedFailure = uiState.lastError?.isNotBlank() == true &&
            currentProblemSignature != approvalBaselineSignature
        if (requestCompleted || operationReportedFailure) {
            approvalInFlightRequestId = null
            approvalInFlightAction = null
            approvalBaselineSignature = ""
        }
    }

    fun submitApproval(request: JoinRequest, action: JoinApprovalAction) {
        if (approvalInFlightRequestId != null) return
        approvalInFlightRequestId = request.requestId
        approvalInFlightAction = action
        approvalBaselineSignature = currentProblemSignature
        onApproval(request, action)
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .testTag("host-dashboard-screen"),
    ) {
        TopAppBar(
            title = { Text(uiState.hostForm.sessionName.ifBlank { "Hosting" }) },
            navigationIcon = {
                IconButton(
                    onClick = onBackRequest,
                    modifier = Modifier.size(WorkflowTouchTarget.minimum),
                ) {
                    Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                }
            },
            actions = {
                IconButton(
                    onClick = onAddSongs,
                    modifier = Modifier.size(WorkflowTouchTarget.minimum),
                ) {
                    Icon(Icons.Filled.Add, contentDescription = "Add songs")
                }
                IconButton(
                    onClick = onManageQueue,
                    modifier = Modifier.size(WorkflowTouchTarget.minimum),
                ) {
                    Icon(Icons.AutoMirrored.Filled.QueueMusic, contentDescription = "Manage queue")
                }
                TextButton(
                    onClick = onInvite,
                    modifier = Modifier.heightIn(min = WorkflowTouchTarget.minimum),
                ) { Text("Invite") }
                IconButton(
                    onClick = { overflowOpen = true },
                    modifier = Modifier.size(WorkflowTouchTarget.minimum),
                ) {
                    Icon(Icons.Filled.MoreVert, contentDescription = "Session actions")
                }
                DropdownMenu(
                    expanded = overflowOpen,
                    onDismissRequest = { overflowOpen = false },
                ) {
                    DropdownMenuItem(
                        text = { Text("Connection help") },
                        onClick = {
                            overflowOpen = false
                            onOpenConnectionHelp()
                        },
                    )
                    DropdownMenuItem(
                        text = { Text("End session", color = MaterialTheme.colorScheme.error) },
                        onClick = {
                            overflowOpen = false
                            onEndSessionRequest()
                        },
                    )
                    if (BuildConfig.DEBUG) {
                        DropdownMenuItem(
                            text = { Text("[Debug] Add demo join") },
                            onClick = {
                                overflowOpen = false
                                onAddDemoJoinRequest()
                            },
                        )
                    }
                }
            },
        )

        Card(modifier = Modifier.fillMaxWidth().padding(horizontal = WorkflowSpacing.large)) {
            Column(
                modifier = Modifier.padding(WorkflowSpacing.large),
                verticalArrangement = Arrangement.spacedBy(WorkflowSpacing.small),
            ) {
                val currentQueueItem = uiState.hostQueue.firstOrNull()
                Text(
                    text = currentQueueItem?.displayName
                        ?: uiState.hostForm.selectedAudio?.displayName
                        ?: "No song selected",
                    style = MaterialTheme.typography.titleLarge,
                    maxLines = 1,
                    overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                    modifier = Modifier.testTag("host-current-song"),
                )
                Text(
                    text = when (uiState.hostPlaybackState) {
                        PlaybackState.PLAYING -> "Streaming"
                        PlaybackState.PAUSED -> "Paused"
                        PlaybackState.BUFFERING -> "Preparing audio"
                        PlaybackState.ERROR -> "Playback problem"
                        else -> "Session ready"
                    },
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                currentQueueItem?.durationMs?.takeIf { it > 0L }?.let { durationMs ->
                    val positionMs = uiState.hostPlaybackPositionMs.coerceIn(0L, durationMs)
                    LinearProgressIndicator(
                        progress = { positionMs.toFloat() / durationMs.toFloat() },
                        modifier = Modifier.fillMaxWidth().testTag("host-playback-progress"),
                    )
                    Text(
                        "${formatPlaybackTime(positionMs)} / ${formatPlaybackTime(durationMs)}",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                val connected = uiState.hostDiagnostics.connectedListenerCount
                Text(if (connected == 1) "1 listener" else "$connected listeners")
                Row(horizontalArrangement = Arrangement.spacedBy(WorkflowSpacing.medium)) {
                    Button(
                        onClick = onPlayPause,
                        enabled = uiState.canStartOrPauseHostPlayback(),
                        modifier = Modifier.weight(1f).heightIn(min = WorkflowTouchTarget.minimum),
                    ) {
                        val playing = uiState.hostPlaybackState == PlaybackState.PLAYING
                        Text(
                            when {
                                playing -> "Pause"
                                uiState.hostPlaybackState == PlaybackState.ERROR -> "Retry"
                                else -> "Play"
                            },
                            maxLines = 1,
                        )
                    }
                    Button(
                        onClick = onStop,
                        enabled = uiState.canStopHostPlayback(),
                        modifier = Modifier.weight(1f).heightIn(min = WorkflowTouchTarget.minimum),
                    ) {
                        Text("Stop", maxLines = 1)
                    }
                }
            }
        }

        LazyColumn(
            modifier = Modifier.weight(1f).fillMaxWidth().testTag("host-dashboard-list"),
            contentPadding = PaddingValues(WorkflowSpacing.large),
            verticalArrangement = Arrangement.spacedBy(WorkflowSpacing.large),
        ) {
            item {
                Card(modifier = Modifier.fillMaxWidth().testTag("host-queue-card")) {
                    Column(
                        modifier = Modifier.padding(WorkflowSpacing.large),
                        verticalArrangement = Arrangement.spacedBy(WorkflowSpacing.small),
                    ) {
                        Text("Queue (${uiState.hostQueue.size})", style = MaterialTheme.typography.titleMedium)
                        Text(
                            uiState.hostQueue.firstOrNull()?.displayName
                                ?: "No songs queued. Add songs, then press Play.",
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }

            item {
                Card(
                    modifier = Modifier
                        .fillMaxWidth()
                        .testTag("host-session-health"),
                ) {
                    Column(
                        modifier = Modifier.padding(WorkflowSpacing.large),
                        verticalArrangement = Arrangement.spacedBy(WorkflowSpacing.small),
                    ) {
                        Text(health.title, style = MaterialTheme.typography.titleMedium)
                        Text(health.detail)
                        if (health.level != SessionHealthLevel.GOOD) {
                            Button(
                                onClick = onOpenConnectionHelp,
                                modifier = Modifier.heightIn(min = WorkflowTouchTarget.minimum),
                            ) {
                                Text("Connection help")
                            }
                        }
                    }
                }
            }

            item {
                PrimaryTabRow(selectedTabIndex = selectedTabIndex) {
                    HostDashboardTab.entries.forEachIndexed { index, tab ->
                        val count = when (tab) {
                            HostDashboardTab.REQUESTS -> uiState.pendingJoinRequests.size
                            HostDashboardTab.CONNECTED -> uiState.approvedListeners.size
                            HostDashboardTab.NEEDS_ATTENTION -> troubledListeners.size
                        }
                        Tab(
                            selected = selectedTabIndex == index,
                            onClick = { selectedTabIndex = index },
                            text = { Text(if (count > 0) "${tab.title} ($count)" else tab.title) },
                        )
                    }
                }
            }

            when (HostDashboardTab.entries[selectedTabIndex]) {
                HostDashboardTab.REQUESTS -> {
                    if (uiState.pendingJoinRequests.isEmpty()) {
                        item { EmptyListenerSection("No one is waiting to join.") }
                    } else {
                        items(uiState.pendingJoinRequests, key = JoinRequest::requestId) { request ->
                            ListenerRequestCard(
                                request = request,
                                approvalLocked = approvalInFlightRequestId != null,
                                inFlightAction = approvalInFlightAction.takeIf {
                                    approvalInFlightRequestId == request.requestId
                                },
                                onApproval = ::submitApproval,
                            )
                        }
                    }
                }

                HostDashboardTab.CONNECTED -> {
                    if (uiState.approvedListeners.isEmpty()) {
                        item { EmptyListenerSection("No listeners are connected yet.") }
                    } else {
                        items(uiState.approvedListeners, key = ListenerInfo::deviceId) { listener ->
                            ConnectedListenerCard(listener, onRemoveListener)
                        }
                    }
                }

                HostDashboardTab.NEEDS_ATTENTION -> {
                    if (troubledListeners.isEmpty()) {
                        item { EmptyListenerSection("No listeners need attention.") }
                    } else {
                        items(troubledListeners, key = ListenerInfo::deviceId) { listener ->
                            ConnectedListenerCard(listener, onRemoveListener)
                        }
                    }
                }
            }
        }
    }
}

private fun formatPlaybackTime(milliseconds: Long): String {
    val totalSeconds = milliseconds / 1_000L
    return "%d:%02d".format(totalSeconds / 60L, totalSeconds % 60L)
}

@Composable
private fun ListenerRequestCard(
    request: JoinRequest,
    approvalLocked: Boolean,
    inFlightAction: JoinApprovalAction?,
    onApproval: (JoinRequest, JoinApprovalAction) -> Unit,
) {
    var elapsedSeconds by remember(request.requestId) {
        mutableLongStateOf(((SystemClock.elapsedRealtime() - request.requestedAtMs) / 1_000L).coerceAtLeast(0L))
    }
    DisposableEffect(request.requestId) {
        val handler = Handler(Looper.getMainLooper())
        val ticker = object : Runnable {
            override fun run() {
                elapsedSeconds = ((SystemClock.elapsedRealtime() - request.requestedAtMs) / 1_000L)
                    .coerceAtLeast(0L)
                handler.postDelayed(this, 1_000L)
            }
        }
        handler.postDelayed(ticker, 1_000L)
        onDispose { handler.removeCallbacks(ticker) }
    }

    Card(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.padding(WorkflowSpacing.large),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Text(request.listenerName, style = MaterialTheme.typography.titleMedium)
            Text(
                waitingDurationLabel(elapsedSeconds),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text(
                "Choose whether this phone is approved once or remembered for future sessions.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (inFlightAction != null) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .testTag("approval-progress-${request.requestId}"),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(WorkflowSpacing.medium),
                ) {
                    CircularProgressIndicator(modifier = Modifier.size(24.dp))
                    Text(approvalProgressLabel(inFlightAction))
                }
            }
            Button(
                onClick = { onApproval(request, JoinApprovalAction.APPROVE_ONCE) },
                enabled = !approvalLocked,
                modifier = Modifier.fillMaxWidth().heightIn(min = WorkflowTouchTarget.minimum),
            ) {
                Text("Approve once")
            }
            Button(
                onClick = { onApproval(request, JoinApprovalAction.ALWAYS_ALLOW) },
                enabled = !approvalLocked,
                modifier = Modifier.fillMaxWidth().heightIn(min = WorkflowTouchTarget.minimum),
            ) {
                Text("Always allow")
            }
            TextButton(
                onClick = { onApproval(request, JoinApprovalAction.REJECT) },
                enabled = !approvalLocked,
                modifier = Modifier.fillMaxWidth().heightIn(min = WorkflowTouchTarget.minimum),
            ) {
                Text("Reject", color = MaterialTheme.colorScheme.error)
            }
        }
    }
}

@Composable
private fun ConnectedListenerCard(
    listener: ListenerInfo,
    onRemoveListener: (String) -> Unit,
) {
    var menuOpen by remember { mutableStateOf(false) }
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.padding(WorkflowSpacing.large),
            verticalArrangement = Arrangement.spacedBy(5.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(listener.displayName, style = MaterialTheme.typography.titleMedium)
                IconButton(onClick = { menuOpen = true }) {
                    Icon(Icons.Filled.MoreVert, contentDescription = "Listener actions")
                }
                DropdownMenu(
                    expanded = menuOpen,
                    onDismissRequest = { menuOpen = false },
                ) {
                    DropdownMenuItem(
                        text = { Text("Disconnect", color = MaterialTheme.colorScheme.error) },
                        onClick = {
                            menuOpen = false
                            onRemoveListener(listener.deviceId)
                        },
                    )
                }
            }
            Text("Connection: ${listener.connectionState.label()}")
            Text("Synchronization: ${listener.syncQuality.label()}")
            Text(
                when (listener.trustState) {
                    TrustState.SESSION_ONLY -> "Approved for this session"
                    TrustState.TRUSTED_PLACEHOLDER -> "Always allowed"
                },
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun EmptyListenerSection(message: String) {
    Text(
        message,
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = WorkflowSpacing.xxLarge),
        style = MaterialTheme.typography.bodyLarge,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

internal fun approvalProgressLabel(action: JoinApprovalAction): String = when (action) {
    JoinApprovalAction.APPROVE_ONCE -> "Sending session approval…"
    JoinApprovalAction.ALWAYS_ALLOW -> "Remembering this phone, then sending approval…"
    JoinApprovalAction.REJECT -> "Sending rejection…"
}

internal fun waitingDurationLabel(elapsedSeconds: Long): String = when {
    elapsedSeconds < 5L -> "Just requested"
    elapsedSeconds < 60L -> "Waiting ${elapsedSeconds}s"
    else -> {
        val minutes = elapsedSeconds / 60L
        "Waiting ${minutes}m"
    }
}

private fun listenerNeedsAttention(listener: ListenerInfo): Boolean =
    listener.connectionState in setOf(
        TransportConnectionState.RETRYING,
        TransportConnectionState.DISCONNECTED,
        TransportConnectionState.FAILED,
    ) || listener.syncQuality in setOf(SyncQualityBadge.POOR, SyncQualityBadge.FAIR)
