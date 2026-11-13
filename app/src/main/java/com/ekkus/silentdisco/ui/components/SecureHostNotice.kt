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
 * Shown on the Android host screens. Since phase 4 (owner decision
 * 2026-11-13) this phone hosts secure transport v4 only: listeners join by
 * scanning its signed QR invitation, and each listener's connection is
 * encrypted. The text says only that, so it stays true before anyone joins.
 */
@Composable
fun SecureHostNotice(modifier: Modifier = Modifier) {
    Card(
        modifier = modifier.fillMaxWidth().testTag("secure-host-notice"),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.secondaryContainer,
        ),
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                text = "Secure connection (v4)",
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.onSecondaryContainer,
                modifier = Modifier.semantics { heading() },
            )
            Text(
                text = "Listeners join this phone by scanning its QR invitation. Controls, " +
                    "clock and audio are encrypted separately for each listener, and phones " +
                    "using the older unencrypted protocol (v3) cannot join.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSecondaryContainer,
            )
        }
    }
}
