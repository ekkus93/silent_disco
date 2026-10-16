package com.ekkus.silentdisco.feature.host

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.AudioFile
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.ui.theme.WorkflowSpacing
import com.ekkus.silentdisco.ui.theme.WorkflowTouchTarget

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HostMusicSetupScreen(
    uiState: AppUiState,
    onBack: () -> Unit,
    onSessionNameChanged: (String) -> Unit,
    onChooseAudio: () -> Unit,
    onRemoveAudio: (Int) -> Unit = {},
    onMoveAudio: (Int, Int) -> Unit = { _, _ -> },
    onNext: () -> Unit,
) {
    val missingItems = buildList {
        if (uiState.hostForm.selectedAudio == null) add("Choose at least one song")
        if (uiState.hostForm.sessionName.isBlank()) add("Enter a session name")
    }
    Column(
        modifier = Modifier
            .fillMaxSize()
            .testTag("host-music-screen"),
    ) {
        TopAppBar(
            title = { Text("Choose music") },
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
                text = "Choose what everyone will hear.",
                style = MaterialTheme.typography.headlineSmall,
            )

            Card(modifier = Modifier.fillMaxWidth()) {
                Column(
                    modifier = Modifier.padding(WorkflowSpacing.xLarge),
                    verticalArrangement = Arrangement.spacedBy(WorkflowSpacing.medium),
                ) {
                    Icon(
                        Icons.Filled.AudioFile,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.primary,
                    )
                    Text(
                        text = if (uiState.hostForm.selectedAudio == null) "No audio selected" else "${uiState.hostForm.additionalAudio.size + 1} songs selected",
                        style = MaterialTheme.typography.titleLarge,
                    )
                    Text(
                        text = if (uiState.hostForm.selectedAudio == null) {
                            "Choose an audio file stored on this phone."
                        } else {
                            "Songs play in this order. Add more songs or adjust the list before continuing."
                        },
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Button(
                        onClick = onChooseAudio,
                        modifier = Modifier
                            .fillMaxWidth()
                            .heightIn(min = WorkflowTouchTarget.minimum)
                            .testTag("host-choose-audio"),
                    ) {
                        Text(if (uiState.hostForm.selectedAudio == null) "Choose songs" else "Add songs")
                    }
                    val playlist = listOfNotNull(uiState.hostForm.selectedAudio) + uiState.hostForm.additionalAudio
                    playlist.forEachIndexed { index, audio ->
                        androidx.compose.material3.ListItem(
                            headlineContent = { Text("${index + 1}. ${audio.displayName}") },
                            trailingContent = {
                                androidx.compose.foundation.layout.Row {
                                    IconButton(
                                        onClick = { onMoveAudio(index, index - 1) },
                                        enabled = index > 0,
                                        modifier = Modifier.sizeIn(
                                            minWidth = WorkflowTouchTarget.minimum,
                                            minHeight = WorkflowTouchTarget.minimum,
                                        ),
                                    ) {
                                        Text("↑", color = MaterialTheme.colorScheme.primary)
                                    }
                                    IconButton(
                                        onClick = { onMoveAudio(index, index + 1) },
                                        enabled = index < playlist.lastIndex,
                                        modifier = Modifier.sizeIn(
                                            minWidth = WorkflowTouchTarget.minimum,
                                            minHeight = WorkflowTouchTarget.minimum,
                                        ),
                                    ) {
                                        Text("↓", color = MaterialTheme.colorScheme.primary)
                                    }
                                    IconButton(
                                        onClick = { onRemoveAudio(index) },
                                        modifier = Modifier.sizeIn(
                                            minWidth = WorkflowTouchTarget.minimum,
                                            minHeight = WorkflowTouchTarget.minimum,
                                        ),
                                    ) {
                                        Text("×", color = MaterialTheme.colorScheme.error)
                                    }
                                }
                            },
                        )
                    }
                }
            }

            OutlinedTextField(
                value = uiState.hostForm.sessionName,
                onValueChange = onSessionNameChanged,
                modifier = Modifier
                    .fillMaxWidth()
                    .testTag("host-session-name"),
                label = { Text("Session name") },
                supportingText = {
                    Text("Listeners will see this name when they look for nearby sessions.")
                },
                singleLine = true,
            )

            if (missingItems.isNotEmpty()) {
                Text(
                    text = missingItems.joinToString(separator = ". ", postfix = "."),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                    modifier = Modifier.testTag("host-music-requirements"),
                )
            }

        }
        Button(
            onClick = onNext,
            enabled = missingItems.isEmpty(),
            modifier = Modifier.fillMaxWidth()
                .padding(horizontal = WorkflowSpacing.large, vertical = WorkflowSpacing.medium)
                .heightIn(min = WorkflowTouchTarget.minimum)
                .testTag("host-music-next"),
        ) {
            Text("Next: Choose access")
        }
    }
}
