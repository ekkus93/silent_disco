package com.ekkus.silentdisco.ui.theme

import androidx.compose.ui.unit.dp

/** Shared spacing steps for reviewed host/client workflow surfaces. */
internal object WorkflowSpacing {
    val xSmall = 4.dp
    val small = 8.dp
    val medium = 12.dp
    val large = 16.dp
    val xLarge = 20.dp
    val xxLarge = 24.dp
    val xxxLarge = 32.dp
}

/** Motion timings paired with the desktop reduced-motion CSS behavior. */
internal object WorkflowMotion {
    const val stateChangeMs = 160
    const val stepChangeMs = 220
}

/** Minimum semantic hit area for actions in the host and client workflows. */
internal object WorkflowTouchTarget {
    val minimum = 48.dp
}
