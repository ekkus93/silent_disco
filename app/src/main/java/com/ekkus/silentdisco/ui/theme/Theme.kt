package com.ekkus.silentdisco.ui.theme

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable

private val SilentDiscoColors = darkColorScheme()

@Composable
fun SilentDiscoTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = SilentDiscoColors,
        content = content,
    )
}
