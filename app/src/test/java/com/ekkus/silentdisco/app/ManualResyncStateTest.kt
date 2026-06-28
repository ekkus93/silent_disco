package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.SessionInfo
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ManualResyncStateTest {

    private val selected = SessionInfo(
        id = "s1",
        name = "Session",
        hostDeviceName = "Host",
        approvalMode = ApprovalMode.MANUAL,
        inviteCodeRequired = false,
    )

    @Test
    fun canManualResync_allowsInitialAndActiveJoinStates() {
        val allowed = listOf(
            ListenerLifecycleState.APPROVED,
            ListenerLifecycleState.CONNECTING,
            ListenerLifecycleState.SYNCING_CLOCK,
            ListenerLifecycleState.BUFFERING,
            ListenerLifecycleState.PLAYING,
            ListenerLifecycleState.RECONNECTING,
            ListenerLifecycleState.DESYNCED,
        )
        allowed.forEach { state ->
            assertTrue(
                "state=$state should allow resync",
                AppUiState(listenerState = state, selectedSession = selected).canManualResync(),
            )
        }
    }

    @Test
    fun canManualResync_rejectsInactiveStates() {
        val rejected = listOf(
            ListenerLifecycleState.IDLE,
            ListenerLifecycleState.SCANNING,
            ListenerLifecycleState.SESSION_SELECTED,
            ListenerLifecycleState.DISCONNECTED,
            ListenerLifecycleState.ERROR,
        )
        rejected.forEach { state ->
            assertFalse(
                "state=$state should reject resync",
                AppUiState(listenerState = state, selectedSession = selected).canManualResync(),
            )
        }
    }

    @Test
    fun canManualResync_falseWithNoSelectedSession() {
        val allowedStates = listOf(
            ListenerLifecycleState.APPROVED,
            ListenerLifecycleState.SYNCING_CLOCK,
            ListenerLifecycleState.PLAYING,
        )
        allowedStates.forEach { state ->
            assertFalse(
                "no selectedSession should reject resync for state=$state",
                AppUiState(listenerState = state, selectedSession = null).canManualResync(),
            )
        }
    }

    @Test
    fun canManualResync_awaitsApprovalStateAllowed() {
        assertTrue(
            AppUiState(
                listenerState = ListenerLifecycleState.AWAITING_APPROVAL,
                selectedSession = selected,
            ).canManualResync(),
        )
    }

    @Test
    fun canManualResync_joinRequestedStateAllowed() {
        assertTrue(
            AppUiState(
                listenerState = ListenerLifecycleState.JOIN_REQUESTED,
                selectedSession = selected,
            ).canManualResync(),
        )
    }
}
