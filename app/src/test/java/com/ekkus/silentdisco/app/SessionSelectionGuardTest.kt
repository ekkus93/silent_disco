package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.SessionInfo
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SessionSelectionGuardTest {

    private fun session(id: String) = SessionInfo(
        id = id,
        name = "Session $id",
        hostDeviceName = "Host",
        approvalMode = ApprovalMode.MANUAL,
        inviteCodeRequired = false,
    )

    @Test
    fun canSelectSession_allowsSameSessionDuringJoin() {
        val sessionA = session("a")
        val state = AppUiState(
            listenerState = ListenerLifecycleState.CONNECTING,
            selectedSession = sessionA,
        )
        assertTrue(state.canSelectSession(sessionA))
    }

    @Test
    fun canSelectSession_rejectsDifferentSessionDuringJoin() {
        val sessionA = session("a")
        val sessionB = session("b")
        val state = AppUiState(
            listenerState = ListenerLifecycleState.CONNECTING,
            selectedSession = sessionA,
        )
        assertFalse(state.canSelectSession(sessionB))
    }

    @Test
    fun canSelectSession_allowsAnySessionWhenIdle() {
        val sessionA = session("a")
        val sessionB = session("b")
        val state = AppUiState(
            listenerState = ListenerLifecycleState.IDLE,
            selectedSession = null,
        )
        assertTrue(state.canSelectSession(sessionA))
        assertTrue(state.canSelectSession(sessionB))
    }

    @Test
    fun canSelectSession_rejectsSwapWhilePlaying() {
        val sessionA = session("a")
        val sessionB = session("b")
        val state = AppUiState(
            listenerState = ListenerLifecycleState.PLAYING,
            selectedSession = sessionA,
        )
        assertFalse(state.canSelectSession(sessionB))
    }

    @Test
    fun canSelectSession_allowsSelectAfterDisconnect() {
        val sessionA = session("a")
        val sessionB = session("b")
        val state = AppUiState(
            listenerState = ListenerLifecycleState.DISCONNECTED,
            selectedSession = sessionA,
        )
        assertTrue(state.canSelectSession(sessionB))
    }

    @Test
    fun isJoinInProgress_trueForActiveJoinStates() {
        val joinStates = listOf(
            ListenerLifecycleState.JOIN_REQUESTED,
            ListenerLifecycleState.AWAITING_APPROVAL,
            ListenerLifecycleState.APPROVED,
            ListenerLifecycleState.CONNECTING,
            ListenerLifecycleState.SYNCING_CLOCK,
            ListenerLifecycleState.BUFFERING,
            ListenerLifecycleState.PLAYING,
            ListenerLifecycleState.RECONNECTING,
            ListenerLifecycleState.DESYNCED,
        )
        joinStates.forEach { state ->
            assertTrue(
                "state=$state should be join-in-progress",
                AppUiState(listenerState = state).isJoinInProgress(),
            )
        }
    }

    @Test
    fun isJoinInProgress_falseForInactiveStates() {
        val inactive = listOf(
            ListenerLifecycleState.IDLE,
            ListenerLifecycleState.SCANNING,
            ListenerLifecycleState.SESSION_SELECTED,
            ListenerLifecycleState.DISCONNECTED,
            ListenerLifecycleState.ERROR,
        )
        inactive.forEach { state ->
            assertFalse(
                "state=$state should not be join-in-progress",
                AppUiState(listenerState = state).isJoinInProgress(),
            )
        }
    }
}
