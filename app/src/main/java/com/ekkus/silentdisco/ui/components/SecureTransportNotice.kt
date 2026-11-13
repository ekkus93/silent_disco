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
 * Shown instead of [QrRequiredNotice] when the host uses secure
 * transport (v4). Such a host can only be joined through its signed
 * invitation: the connection is encrypted per listener and the host must hold
 * the invitation's key, and a plain join is refused before connecting. The
 * text says only that, so it stays true before a connection exists.
 */
@Composable
fun SecureTransportNotice(modifier: Modifier = Modifier) {
    Card(
        modifier = modifier.fillMaxWidth().testTag("secure-transport-notice"),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.secondaryContainer,
        ),
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                text = "Secure connection (v4, experimental)",
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.onSecondaryContainer,
                modifier = Modifier.semantics { heading() },
            )
            Text(
                text = "This host uses secure transport. Controls, clock and audio are encrypted " +
                    "for this phone, and the host must hold the key from its signed invitation. " +
                    "Without that invitation, this phone cannot join it.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSecondaryContainer,
            )
        }
    }
}
