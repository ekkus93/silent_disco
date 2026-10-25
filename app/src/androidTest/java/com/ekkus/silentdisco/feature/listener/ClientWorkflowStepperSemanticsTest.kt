package com.ekkus.silentdisco.feature.listener

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.foundation.layout.width
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.getTextLayoutResult
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Emulator qualification: semantics survive every live join-stage update. */
@RunWith(AndroidJUnit4::class)
class ClientWorkflowStepperSemanticsTest {
    @get:Rule val compose = createComposeRule()

    @Test
    fun findConnectListenTransitionsKeepReadableAccessibilitySemantics() {
        var stage by mutableStateOf(ClientWorkflowStage.FIND)
        compose.setContent {
            SilentDiscoTheme {
                ClientWorkflowStepper(stage = stage)
            }
        }

        fun assertStage(expected: ClientWorkflowStage) {
            compose.onNodeWithTag("client-workflow-stepper", useUnmergedTree = true)
                .assert(SemanticsMatcher.expectValue(
                    SemanticsProperties.ContentDescription,
                    listOf("Join a disco: " + expected.label + " stage"),
                ))
            ClientWorkflowStage.entries.forEachIndexed { index, item ->
                compose.onNodeWithTag(
                    "client-workflow-step-" + item.name.lowercase(),
                    useUnmergedTree = true,
                ).assertExists()
                    .assert(SemanticsMatcher.expectValue(
                        SemanticsProperties.StateDescription,
                        when {
                            index == expected.ordinal -> "Current step"
                            index < expected.ordinal -> "Complete"
                            else -> "Not started"
                        },
                    ))
            }
        }

        assertStage(ClientWorkflowStage.FIND)
        compose.runOnIdle { stage = ClientWorkflowStage.CONNECT }
        assertStage(ClientWorkflowStage.CONNECT)
        compose.runOnIdle { stage = ClientWorkflowStage.LISTEN }
        assertStage(ClientWorkflowStage.LISTEN)
    }

    @Test
    fun compactLargeTextWrapsVisibleStageLabelsWithoutClipping() {
        compose.setContent {
            CompositionLocalProvider(LocalDensity provides Density(1f, 2f)) {
                SilentDiscoTheme {
                    ClientWorkflowStepper(
                        stage = ClientWorkflowStage.CONNECT,
                        modifier = Modifier.width(240.dp),
                    )
                }
            }
        }

        val connect = compose.onNodeWithTag(
            "client-workflow-step-connect",
            useUnmergedTree = true,
        )
        val layout = connect.getTextLayoutResult()
        assertTrue("Connect must wrap at 2x font scale in a compact viewport", layout.lineCount > 1)
        assertFalse("Visible stage text must not overflow or be ellipsized", layout.hasVisualOverflow)
        connect.assert(SemanticsMatcher.expectValue(
            SemanticsProperties.StateDescription,
            "Current step",
        ))
    }
}
