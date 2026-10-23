package com.ekkus.silentdisco.ui.theme

import org.junit.Assert.assertEquals
import org.junit.Test

class WorkflowMotionTest {
    @Test
    fun zeroSystemMotionScaleRemovesDecorativeWorkflowTransitions() {
        assertEquals(0, workflowStateChangeDurationMs(0f))
        assertEquals(0, workflowStateChangeDurationMs(-1f))
        assertEquals(0, workflowStateChangeDurationMs(Float.NaN))
    }

    @Test
    fun defaultAndSlowedSystemScaleKeepUserConfiguredComposeMotion() {
        assertEquals(WorkflowMotion.stateChangeMs, workflowStateChangeDurationMs(1f))
        assertEquals(WorkflowMotion.stateChangeMs, workflowStateChangeDurationMs(2f))
    }
}
