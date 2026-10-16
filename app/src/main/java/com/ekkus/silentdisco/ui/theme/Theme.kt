package com.ekkus.silentdisco.ui.theme

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.foundation.shape.RoundedCornerShape

internal val SilentDiscoColors = darkColorScheme(
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
    tertiary = Color(0xFFFFB86B),
    onTertiary = Color(0xFF452B00),
    tertiaryContainer = Color(0xFF624000),
    onTertiaryContainer = Color(0xFFFFDDB4),
    error = Color(0xFFFFB4AB),
    onError = Color(0xFF690005),
    errorContainer = Color(0xFF93000A),
    onErrorContainer = Color(0xFFFFDAD6),
    outline = Color(0xFF948FA6),
    outlineVariant = Color(0xFF494458),
)

private val SilentDiscoShapes = Shapes(
    extraSmall = RoundedCornerShape(8.dp),
    small = RoundedCornerShape(12.dp),
    medium = RoundedCornerShape(16.dp),
    large = RoundedCornerShape(24.dp),
    extraLarge = RoundedCornerShape(32.dp),
)

private val SilentDiscoTypography = Typography(
    displaySmall = TextStyle(
        fontSize = 36.sp,
        fontWeight = FontWeight.Bold,
        letterSpacing = (-0.5).sp,
        lineHeight = 44.sp,
    ),
    headlineMedium = TextStyle(
        fontSize = 28.sp,
        fontWeight = FontWeight.SemiBold,
        letterSpacing = (-0.25).sp,
        lineHeight = 36.sp,
    ),
    titleLarge = TextStyle(
        fontSize = 22.sp,
        fontWeight = FontWeight.SemiBold,
        lineHeight = 28.sp,
    ),
)

@Composable
fun SilentDiscoTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = SilentDiscoColors,
        typography = SilentDiscoTypography,
        shapes = SilentDiscoShapes,
        content = content,
    )
}
