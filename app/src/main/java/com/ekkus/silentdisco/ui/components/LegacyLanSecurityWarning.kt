package com.ekkus.silentdisco.ui.components

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp

/**
 * Protocol-v3 TCP control and UDP PCM are plaintext and are not bound to a
 * signed QR or verified peer handshake. Keep this notice visible on BOTH
 * sides of the production workflow until secure transport actually ships.
 */
@Composable
fun LegacyLanSecurityWarning(modifier: Modifier = Modifier) {
    Card(
        modifier = modifier.fillMaxWidth().testTag("legacy-lan-security-warning"),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.errorContainer,
        ),
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                text = "Local network connection is not secured",
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.onErrorContainer,
                modifier = Modifier.semantics { heading() },
            )
            Text(
                text = "This version does not encrypt or authenticate live session traffic. " +
                    "Others on the same Wi-Fi may read or alter invite codes, controls, and audio. " +
                    "A signed QR or approved-device name does not protect the connection. " +
                    "Only use a private network you trust.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onErrorContainer,
            )
        }
    }
}
