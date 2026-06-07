package com.ekkus.silentdisco.ui.theme

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

private val SilentDiscoColors = darkColorScheme(
    primary = Color(0xFF9C7DFF),
    onPrimary = Color(0xFF1A0070),
    primaryContainer = Color(0xFF3B0099),
    onPrimaryContainer = Color(0xFFE8DEFF),
    secondary = Color(0xFF00E5CC),
    onSecondary = Color(0xFF003731),
    secondaryContainer = Color(0xFF00504A),
    onSecondaryContainer = Color(0xFF7FF8E9),
    background = Color(0xFF0F0E1A),
    onBackground = Color(0xFFE6E1F9),
    surface = Color(0xFF1A1929),
    onSurface = Color(0xFFE6E1F9),
    surfaceVariant = Color(0xFF2D2B3E),
    onSurfaceVariant = Color(0xFFCAC4DD),
)

@Composable
fun SilentDiscoTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = SilentDiscoColors,
        content = content,
    )
}
