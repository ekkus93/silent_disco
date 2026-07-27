package com.ekkus.silentdisco.feature.p2

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.core.rust.P2ValidatedInvitation
import com.google.zxing.BarcodeFormat
import com.journeyapps.barcodescanner.BarcodeEncoder

@Composable
fun QrHostInvitationDialog(
    payload: String,
    onDismiss: () -> Unit,
) {
    val bitmapResult = remember(payload) {
        runCatching {
            BarcodeEncoder().encodeBitmap(payload, BarcodeFormat.QR_CODE, 720, 720)
        }
    }
    AlertDialog(
        modifier = Modifier.testTag("host-qr-dialog"),
        onDismissRequest = onDismiss,
        title = { Text("Scan to join securely") },
        text = {
            Column(
                modifier = Modifier.fillMaxWidth(),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                bitmapResult.fold(
                    onSuccess = { bitmap ->
                        Image(
                            bitmap = bitmap.asImageBitmap(),
                            contentDescription = "Signed Silent Disco join QR code",
                            modifier = Modifier
                                .size(280.dp)
                                .testTag("host-qr-image"),
                        )
                    },
                    onFailure = {
                        Text(
                            "The signed invitation was created, but Android could not render its QR image.",
                            color = MaterialTheme.colorScheme.error,
                            modifier = Modifier.testTag("host-qr-render-error"),
                        )
                    },
                )
                Text(
                    "This invitation is signed by this phone and expires after five minutes. It may be used once per listener phone.",
                    style = MaterialTheme.typography.bodyMedium,
                )
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text("Done") }
        },
    )
}

@Composable
fun VerifiedQrInvitationDialog(
    invitation: P2ValidatedInvitation,
    alreadyTrusted: Boolean,
    onDismiss: () -> Unit,
    onJoinOnce: () -> Unit,
    onTrustAndJoin: () -> Unit,
) {
    AlertDialog(
        modifier = Modifier.testTag("verified-qr-dialog"),
        onDismissRequest = onDismiss,
        title = { Text("Verified invitation") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text(invitation.sessionName, style = MaterialTheme.typography.titleLarge)
                Text("Hosted by ${invitation.hostName}")
                Text(
                    if (alreadyTrusted) {
                        "This signature matches a host key you previously trusted."
                    } else {
                        "The QR signature is valid. The host is verified for this invitation only unless you choose to trust its public key."
                    },
                    modifier = Modifier.testTag("verified-host-status"),
                )
                Text(
                    "Key fingerprint: ${invitation.hostFingerprint.take(16)}…",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                if (invitation.inviteCode != null) {
                    Text("The signed invitation includes the required invite code.")
                }
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        },
        confirmButton = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(
                    onClick = onJoinOnce,
                    modifier = Modifier
                        .fillMaxWidth()
                        .testTag("verified-qr-join-once"),
                ) {
                    Text("Join once")
                }
                Button(
                    onClick = onTrustAndJoin,
                    modifier = Modifier
                        .fillMaxWidth()
                        .testTag("verified-qr-trust-join"),
                ) {
                    Text(if (alreadyTrusted) "Join trusted host" else "Trust host and join")
                }
            }
        },
    )
}
