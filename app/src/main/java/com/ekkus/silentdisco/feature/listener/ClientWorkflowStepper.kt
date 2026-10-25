package com.ekkus.silentdisco.feature.listener

import android.animation.ValueAnimator
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import com.ekkus.silentdisco.ui.theme.WorkflowSpacing
import com.ekkus.silentdisco.ui.theme.workflowStateChangeDurationMs

internal enum class ClientWorkflowStage(val label: String) {
    FIND("Find"),
    CONNECT("Connect"),
    LISTEN("Listen"),
}

@Composable
internal fun ClientWorkflowStepper(
    stage: ClientWorkflowStage,
    modifier: Modifier = Modifier,
) {
    val motionDuration = workflowStateChangeDurationMs(if (ValueAnimator.areAnimatorsEnabled()) 1f else 0f)
    Row(
        modifier = modifier
            .fillMaxWidth()
            .padding(horizontal = WorkflowSpacing.large, vertical = WorkflowSpacing.small)
            .semantics {
                contentDescription = "Join a disco: ${stage.label} stage"
            }
            .testTag("client-workflow-stepper"),
        horizontalArrangement = Arrangement.spacedBy(WorkflowSpacing.small),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        ClientWorkflowStage.entries.forEachIndexed { index, item ->
            val currentIndex = stage.ordinal
            val isCurrent = index == currentIndex
            val isComplete = index < currentIndex
            val label = if (isComplete) "✓ ${item.label}" else "${index + 1} ${item.label}"
            val targetColor = if (isCurrent || isComplete) {
                MaterialTheme.colorScheme.primary
            } else {
                MaterialTheme.colorScheme.onSurfaceVariant
            }
            val color by animateColorAsState(
                targetValue = targetColor,
                animationSpec = tween(durationMillis = motionDuration),
                label = "client_workflow_step_${item.name.lowercase()}_color",
            )
            Text(
                text = label,
                modifier = Modifier
                    .weight(1f)
                    .semantics {
                        contentDescription = "Step ${index + 1}: ${item.label}"
                        stateDescription = when {
                            isCurrent -> "Current step"
                            isComplete -> "Complete"
                            else -> "Not started"
                        }
                    }
                    .testTag("client-workflow-step-${item.name.lowercase()}"),
                style = MaterialTheme.typography.labelLarge,
                fontWeight = if (isCurrent) FontWeight.Bold else FontWeight.Normal,
                color = color,
                textAlign = TextAlign.Center,
                // Allow labels to wrap at large system font scales instead of
                // silently truncating the visible stage while TalkBack reads it.
            )
        }
    }
}
