package com.ekkus.silentdisco.ui.components

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Error
import androidx.compose.material.icons.filled.HourglassTop
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp

enum class StatusTone {
    POSITIVE,
    ATTENTION,
    CRITICAL,
    NEUTRAL,
    IN_PROGRESS,
}

@Composable
fun StatusBadge(
    text: String,
    tone: StatusTone,
    semanticLabel: String = text,
    modifier: Modifier = Modifier,
) {
    val icon: ImageVector = when (tone) {
        StatusTone.POSITIVE -> Icons.Filled.CheckCircle
        StatusTone.ATTENTION -> Icons.Filled.Warning
        StatusTone.CRITICAL -> Icons.Filled.Error
        StatusTone.NEUTRAL -> Icons.Filled.Info
        StatusTone.IN_PROGRESS -> Icons.Filled.HourglassTop
    }
    val containerColor = when (tone) {
        StatusTone.POSITIVE -> MaterialTheme.colorScheme.primaryContainer
        StatusTone.ATTENTION -> MaterialTheme.colorScheme.tertiaryContainer
        StatusTone.CRITICAL -> MaterialTheme.colorScheme.errorContainer
        StatusTone.NEUTRAL -> MaterialTheme.colorScheme.surfaceVariant
        StatusTone.IN_PROGRESS -> MaterialTheme.colorScheme.secondaryContainer
    }
    val contentColor = when (tone) {
        StatusTone.POSITIVE -> MaterialTheme.colorScheme.onPrimaryContainer
        StatusTone.ATTENTION -> MaterialTheme.colorScheme.onTertiaryContainer
        StatusTone.CRITICAL -> MaterialTheme.colorScheme.onErrorContainer
        StatusTone.NEUTRAL -> MaterialTheme.colorScheme.onSurfaceVariant
        StatusTone.IN_PROGRESS -> MaterialTheme.colorScheme.onSecondaryContainer
    }

    Surface(
        modifier = modifier.semantics { contentDescription = semanticLabel },
        shape = RoundedCornerShape(999.dp),
        color = containerColor,
        contentColor = contentColor,
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp),
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(icon, contentDescription = null)
            Text(text, style = MaterialTheme.typography.labelLarge)
        }
    }
}
