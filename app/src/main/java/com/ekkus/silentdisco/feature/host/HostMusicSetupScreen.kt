package com.ekkus.silentdisco.feature.host

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
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
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HostMusicSetupScreen(
    uiState: AppUiState,
    onBack: () -> Unit,
    onSessionNameChanged: (String) -> Unit,
    onChooseAudio: () -> Unit,
    onNext: () -> Unit,
) {
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
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(
                text = "Choose what everyone will hear.",
                style = MaterialTheme.typography.headlineSmall,
            )

            Card(modifier = Modifier.fillMaxWidth()) {
                Column(
                    modifier = Modifier.padding(20.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Icon(
                        Icons.Filled.AudioFile,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.primary,
                    )
                    Text(
                        text = uiState.hostForm.selectedAudio?.displayName ?: "No audio selected",
                        style = MaterialTheme.typography.titleLarge,
                    )
                    Text(
                        text = if (uiState.hostForm.selectedAudio == null) {
                            "Choose an audio file stored on this phone."
                        } else {
                            "This file will be streamed to approved listeners."
                        },
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Button(
                        onClick = onChooseAudio,
                        modifier = Modifier
                            .fillMaxWidth()
                            .testTag("host-choose-audio"),
                    ) {
                        Text(if (uiState.hostForm.selectedAudio == null) "Choose audio" else "Choose different audio")
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

            val missingItems = buildList {
                if (uiState.hostForm.selectedAudio == null) add("Choose an audio file")
                if (uiState.hostForm.sessionName.isBlank()) add("Enter a session name")
            }
            if (missingItems.isNotEmpty()) {
                Text(
                    text = missingItems.joinToString(separator = ". ", postfix = "."),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                    modifier = Modifier.testTag("host-music-requirements"),
                )
            }

            Button(
                onClick = onNext,
                enabled = missingItems.isEmpty(),
                modifier = Modifier
                    .fillMaxWidth()
                    .testTag("host-music-next"),
            ) {
                Text("Next: Choose access")
            }
        }
    }
}
