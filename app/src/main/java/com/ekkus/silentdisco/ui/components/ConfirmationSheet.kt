package com.ekkus.silentdisco.ui.components

import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.testTag

@Composable
fun ConfirmationSheet(
    visible: Boolean,
    title: String,
    detail: String,
    safeActionLabel: String,
    destructiveActionLabel: String,
    onDismiss: () -> Unit,
    onConfirm: () -> Unit,
    testTag: String,
) {
    var confirming by rememberSaveable(testTag) { mutableStateOf(false) }
    val safeActionFocusRequester = remember(testTag) { FocusRequester() }

    LaunchedEffect(visible) {
        confirming = false
        if (visible) {
            // AlertDialog content is attached in a separate window. Wait for that window to
            // complete a frame before requesting focus so the safe action wins deterministically.
            withFrameNanos { }
            safeActionFocusRequester.requestFocus()
        }
    }

    if (!visible) return

    AlertDialog(
        modifier = Modifier.testTag(testTag),
        onDismissRequest = {
            if (!confirming) onDismiss()
        },
        title = { Text(title) },
        text = { Text(detail) },
        dismissButton = {
            TextButton(
                onClick = onDismiss,
                enabled = !confirming,
                modifier = Modifier
                    .focusRequester(safeActionFocusRequester)
                    .testTag("$testTag-safe"),
            ) {
                Text(safeActionLabel)
            }
        },
        confirmButton = {
            TextButton(
                onClick = {
                    if (!confirming) {
                        confirming = true
                        onConfirm()
                    }
                },
                enabled = !confirming,
                modifier = Modifier.testTag("$testTag-destructive"),
            ) {
                Text(
                    destructiveActionLabel,
                    color = MaterialTheme.colorScheme.error,
                )
            }
        },
    )
}
