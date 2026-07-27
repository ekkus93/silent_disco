package com.ekkus.silentdisco.feature.host

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.QrCode
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.core.model.ApprovalMode

@Composable
fun InviteSessionSheet(
    uiState: AppUiState,
    onDismiss: () -> Unit,
    onCopyCode: (String) -> Unit,
    onShareInstructions: (String) -> Unit,
    onShowSignedQr: (() -> Unit)? = null,
) {
    val instructions = inviteInstructions(uiState)
    val inviteCode = uiState.hostForm.inviteCode.takeIf {
        uiState.hostForm.approvalMode == ApprovalMode.INVITE_CODE && it.isNotBlank()
    }

    AlertDialog(
        modifier = Modifier.testTag("invite-session-sheet"),
        onDismissRequest = onDismiss,
        title = { Text("Invite listeners") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(
                    uiState.hostForm.sessionName.ifBlank { "Silent Disco session" },
                    style = MaterialTheme.typography.titleMedium,
                )
                Text(instructions)
                inviteCode?.let { code ->
                    Text(
                        "Invite code: $code",
                        style = MaterialTheme.typography.headlineSmall,
                    )
                    Button(
                        onClick = { onCopyCode(code) },
                        modifier = Modifier
                            .fillMaxWidth()
                            .testTag("invite-copy-code"),
                    ) {
                        Icon(Icons.Filled.ContentCopy, contentDescription = null)
                        Text("Copy code")
                    }
                }
                if (onShowSignedQr != null) {
                    Button(
                        onClick = onShowSignedQr,
                        modifier = Modifier
                            .fillMaxWidth()
                            .testTag("invite-show-qr"),
                    ) {
                        Icon(Icons.Filled.QrCode, contentDescription = null)
                        Text("Show signed QR code")
                    }
                    Text(
                        "The QR invitation verifies this host phone's public key and expires after five minutes.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                OutlinedButton(
                    onClick = { onShareInstructions(instructions) },
                    modifier = Modifier
                        .fillMaxWidth()
                        .testTag("invite-share-instructions"),
                ) {
                    Icon(Icons.Filled.Share, contentDescription = null)
                    Text("Share instructions")
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text("Done") }
        },
    )
}

internal fun inviteInstructions(uiState: AppUiState): String {
    val sessionName = uiState.hostForm.sessionName.ifBlank { "Silent Disco session" }
    val accessInstruction = when (uiState.hostForm.approvalMode) {
        ApprovalMode.MANUAL -> "The host will approve your request."
        ApprovalMode.INVITE_CODE -> {
            val code = uiState.hostForm.inviteCode
            if (code.isBlank()) "Ask the host for the invite code." else "Use invite code $code."
        }
        ApprovalMode.TRUSTED_DEVICES_PLACEHOLDER ->
            "Only phones previously marked as always allowed can join."
    }
    return "Open Silent Disco, choose Join music, and select ‘$sessionName’. $accessInstruction"
}
