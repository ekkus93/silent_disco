package com.ekkus.silentdisco.feature.startup

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.StorageInitializationState

@Composable
fun StartupGateScreen(
    uiState: AppUiState,
    onRetry: () -> Unit,
    onShareSupportReport: () -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(24.dp)
            .testTag("startup-screen"),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        when (uiState.storageState) {
            StorageInitializationState.INITIALIZING -> StartupLoading()
            StorageInitializationState.READY -> StartupReady()
            StorageInitializationState.RECOVERABLE_FAILURE -> StartupFailure(
                title = "Local app data is temporarily unavailable",
                detail = "Silent Disco could not open local app data yet. Retry before hosting or joining.",
                technicalDetail = uiState.storageError,
                retryable = true,
                onRetry = onRetry,
                onShareSupportReport = onShareSupportReport,
            )
            StorageInitializationState.FATAL_FAILURE -> StartupFailure(
                title = "Local app data could not be opened",
                detail = "Silent Disco cannot safely continue. Share a support report for troubleshooting.",
                technicalDetail = uiState.storageError,
                retryable = false,
                onRetry = onRetry,
                onShareSupportReport = onShareSupportReport,
            )
        }
    }
}

@Composable
private fun StartupLoading() {
    CircularProgressIndicator(
        modifier = Modifier
            .semantics { contentDescription = "Opening local app data" }
            .testTag("startup-loading"),
    )
    Spacer(Modifier.height(24.dp))
    Text(
        text = "Getting Silent Disco ready…",
        modifier = Modifier.semantics { heading() },
        style = MaterialTheme.typography.headlineSmall,
    )
    Spacer(Modifier.height(8.dp))
    Text(
        text = "Opening local app data",
        style = MaterialTheme.typography.bodyLarge,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

@Composable
private fun StartupReady() {
    CircularProgressIndicator(
        modifier = Modifier
            .semantics { contentDescription = "Startup complete" }
            .testTag("startup-ready"),
    )
    Spacer(Modifier.height(24.dp))
    Text(
        text = "Silent Disco is ready",
        modifier = Modifier.semantics { heading() },
        style = MaterialTheme.typography.headlineSmall,
    )
}

@Composable
private fun StartupFailure(
    title: String,
    detail: String,
    technicalDetail: String?,
    retryable: Boolean,
    onRetry: () -> Unit,
    onShareSupportReport: () -> Unit,
) {
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .testTag(if (retryable) "startup-recoverable" else "startup-fatal"),
    ) {
        Column(
            modifier = Modifier.padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                title,
                modifier = Modifier.semantics { heading() },
                style = MaterialTheme.typography.headlineSmall,
            )
            Text(detail, style = MaterialTheme.typography.bodyLarge)
            technicalDetail?.takeIf(String::isNotBlank)?.let {
                Text(
                    text = it,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            if (retryable) {
                Button(
                    onClick = onRetry,
                    modifier = Modifier
                        .fillMaxWidth()
                        .testTag("startup-retry"),
                ) {
                    Text("Retry")
                }
            }
            TextButton(
                onClick = onShareSupportReport,
                modifier = Modifier
                    .fillMaxWidth()
                    .testTag("startup-share-support"),
            ) {
                Text("Share support report")
            }
        }
    }
}
