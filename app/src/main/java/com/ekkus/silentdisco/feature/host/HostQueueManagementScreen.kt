package com.ekkus.silentdisco.feature.host

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.HostQueueItem
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.ui.theme.WorkflowSpacing
import com.ekkus.silentdisco.ui.theme.WorkflowTouchTarget

@Composable
@OptIn(ExperimentalMaterial3Api::class)
fun HostQueueManagementScreen(
    uiState: AppUiState,
    onBack: () -> Unit,
    onAddSongs: () -> Unit,
    onReorderQueue: (List<String>) -> Unit,
    onRemoveQueueEntry: (String) -> Unit,
) {
    val queue = uiState.hostQueue
    val currentLocked = uiState.hostPlaybackState in setOf(
        PlaybackState.PLAYING,
        PlaybackState.PAUSED,
        PlaybackState.BUFFERING,
        PlaybackState.UNDERRUN,
    )

    Scaffold(
        modifier = Modifier.fillMaxSize().testTag("host-queue-management-screen"),
        topBar = {
            TopAppBar(
                title = { Text("Manage queue (${queue.size})") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        },
        bottomBar = {
            Button(
                onClick = onAddSongs,
                modifier = Modifier.fillMaxWidth()
                    .padding(horizontal = WorkflowSpacing.large, vertical = WorkflowSpacing.medium)
                    .heightIn(min = WorkflowTouchTarget.minimum)
                    .testTag("host-queue-add-songs"),
            ) {
                Icon(Icons.Filled.Add, contentDescription = null)
                Text("Add songs")
            }
        },
    ) { insets ->
        Column(
            modifier = Modifier.fillMaxSize().padding(insets).padding(horizontal = WorkflowSpacing.large),
            verticalArrangement = Arrangement.spacedBy(WorkflowSpacing.medium),
        ) {
            val current = queue.firstOrNull()
            val failedCurrent = current != null && uiState.hostPlaybackState == PlaybackState.ERROR
            Card(modifier = Modifier.fillMaxWidth().testTag("host-queue-current")) {
                Column(modifier = Modifier.padding(WorkflowSpacing.large), verticalArrangement = Arrangement.spacedBy(WorkflowSpacing.xSmall)) {
                    Text(if (failedCurrent) "Song needs attention" else "Current song", style = MaterialTheme.typography.titleSmall)
                    Text(
                        current?.displayName ?: "No song is playing. Add a song, then press Play.",
                        style = MaterialTheme.typography.bodyLarge,
                    )
                    if (current != null) {
                        Text(
                            when {
                                currentLocked -> "Playing now"
                                failedCurrent -> "Playback stopped. Retry or remove this song."
                                else -> "First in queue"
                            },
                            color = MaterialTheme.colorScheme.primary,
                        )
                        if (failedCurrent) {
                            TextButton(
                                onClick = { onRemoveQueueEntry(current.entryId) },
                                modifier = Modifier.testTag("host-queue-remove-failed-current"),
                            ) { Text("Remove failed song") }
                        }
                    }
                }
            }

            Text("Upcoming (${(queue.size - if (current == null) 0 else 1).coerceAtLeast(0)})")
            if (queue.size <= 1) {
                Text(
                    "No upcoming songs. Add songs to keep the queue going.",
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            } else {
                LazyColumn(
                    modifier = Modifier.weight(1f).fillMaxWidth().testTag("host-queue-upcoming-list"),
                    contentPadding = PaddingValues(bottom = WorkflowSpacing.medium),
                    verticalArrangement = Arrangement.spacedBy(WorkflowSpacing.small),
                ) {
                    itemsIndexed(queue.drop(1), key = { _, entry -> entry.entryId }) { upcomingIndex, entry ->
                        QueueEntryRow(
                            entry = entry,
                            index = upcomingIndex + 1,
                            queue = queue,
                            onReorderQueue = onReorderQueue,
                            onRemoveQueueEntry = onRemoveQueueEntry,
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun QueueEntryRow(
    entry: HostQueueItem,
    index: Int,
    queue: List<HostQueueItem>,
    onReorderQueue: (List<String>) -> Unit,
    onRemoveQueueEntry: (String) -> Unit,
) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(start = WorkflowSpacing.medium, end = WorkflowSpacing.xSmall, top = WorkflowSpacing.xSmall, bottom = WorkflowSpacing.xSmall),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("${index + 1}. ${entry.displayName}", modifier = Modifier.weight(1f))
            TextButton(
                onClick = { reorderQueue(queue, index, index - 1, onReorderQueue) },
                enabled = index > 1,
                modifier = Modifier
                    .heightIn(min = WorkflowTouchTarget.minimum)
                    .semantics { contentDescription = "Move ${entry.displayName} up" }
                    .testTag("host-queue-move-up-${entry.entryId}"),
            ) { Text("↑") }
            TextButton(
                onClick = { reorderQueue(queue, index, index + 1, onReorderQueue) },
                enabled = index < queue.lastIndex,
                modifier = Modifier
                    .heightIn(min = WorkflowTouchTarget.minimum)
                    .semantics { contentDescription = "Move ${entry.displayName} down" }
                    .testTag("host-queue-move-down-${entry.entryId}"),
            ) { Text("↓") }
            TextButton(
                onClick = { onRemoveQueueEntry(entry.entryId) },
                modifier = Modifier
                    .heightIn(min = WorkflowTouchTarget.minimum)
                    .semantics { contentDescription = "Remove ${entry.displayName}" }
                    .testTag("host-queue-remove-${entry.entryId}"),
            ) { Text("Remove") }
        }
    }
}

private fun reorderQueue(
    queue: List<HostQueueItem>,
    from: Int,
    to: Int,
    onReorderQueue: (List<String>) -> Unit,
) {
    if (from !in queue.indices || to !in queue.indices || from == 0 || to == 0) return
    val ids = queue.map { it.entryId }.toMutableList()
    val moved = ids.removeAt(from)
    ids.add(to, moved)
    onReorderQueue(ids)
}
