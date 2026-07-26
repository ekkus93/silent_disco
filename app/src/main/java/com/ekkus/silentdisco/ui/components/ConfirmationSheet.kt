package com.ekkus.silentdisco.ui.components

import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
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
    if (!visible) return

    AlertDialog(
        modifier = Modifier.testTag(testTag),
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = { Text(detail) },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text(safeActionLabel)
            }
        },
        confirmButton = {
            TextButton(onClick = onConfirm) {
                Text(
                    destructiveActionLabel,
                    color = MaterialTheme.colorScheme.error,
                )
            }
        },
    )
}
