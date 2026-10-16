package com.ekkus.silentdisco.ui.theme

import androidx.compose.ui.graphics.Color
import com.google.common.truth.Truth.assertWithMessage
import kotlin.math.pow
import org.junit.Test

class WorkflowThemeContrastTest {
    @Test
    fun workflowTextRolesMeetWcagAaContrastAgainstTheirSurfaces() {
        val pairs = listOf(
            "on background" to (SilentDiscoColors.onBackground to SilentDiscoColors.background),
            "on surface" to (SilentDiscoColors.onSurface to SilentDiscoColors.surface),
            "on surface variant" to (SilentDiscoColors.onSurfaceVariant to SilentDiscoColors.surface),
            "on surface variant card" to
                (SilentDiscoColors.onSurfaceVariant to SilentDiscoColors.surfaceVariant),
            "on primary" to (SilentDiscoColors.onPrimary to SilentDiscoColors.primary),
            "on primary container" to
                (SilentDiscoColors.onPrimaryContainer to SilentDiscoColors.primaryContainer),
            "on secondary container" to
                (SilentDiscoColors.onSecondaryContainer to SilentDiscoColors.secondaryContainer),
            "on tertiary" to (SilentDiscoColors.onTertiary to SilentDiscoColors.tertiary),
            "on tertiary container" to
                (SilentDiscoColors.onTertiaryContainer to SilentDiscoColors.tertiaryContainer),
            "on error container" to
                (SilentDiscoColors.onErrorContainer to SilentDiscoColors.errorContainer),
        )

        pairs.forEach { (name, colors) ->
            assertWithMessage("$name contrast")
                .that(contrastRatio(colors.first, colors.second))
                .isAtLeast(4.5)
        }
    }
}

private fun contrastRatio(foreground: Color, background: Color): Double {
    val first = relativeLuminance(foreground)
    val second = relativeLuminance(background)
    return (maxOf(first, second) + 0.05) / (minOf(first, second) + 0.05)
}

private fun relativeLuminance(color: Color): Double {
    fun linearize(channel: Float): Double {
        val value = channel.toDouble()
        return if (value <= 0.04045) value / 12.92 else ((value + 0.055) / 1.055).pow(2.4)
    }

    return 0.2126 * linearize(color.red) +
        0.7152 * linearize(color.green) +
        0.0722 * linearize(color.blue)
}
