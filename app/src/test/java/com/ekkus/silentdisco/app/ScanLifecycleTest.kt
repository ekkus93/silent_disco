package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.SessionInfo
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ScanLifecycleTest {
    @Test
    fun isScanning_defaultsFalse() {
        val state = AppUiState()
        assertFalse(state.isScanning)
    }

    @Test
    fun isScanning_canBeSetTrue() {
        val state = AppUiState(isScanning = true)
        assertTrue(state.isScanning)
    }

    @Test
    fun scanWindowMs_defaultsTo3Seconds() {
        val settings = TuningSettings()
        assertEquals(3_000L, settings.scanWindowMs)
    }

    @Test
    fun scanWindowMs_normalizedToValidRange() {
        val tooShort = TuningSettings(scanWindowMs = 500L)
        assertEquals(1_000L, tooShort.normalized().scanWindowMs)

        val tooLong = TuningSettings(scanWindowMs = 15_000L)
        assertEquals(10_000L, tooLong.normalized().scanWindowMs)

        val valid = TuningSettings(scanWindowMs = 5_000L)
        assertEquals(5_000L, valid.normalized().scanWindowMs)
    }

    @Test
    fun scanWindowMs_adjustmentStep() {
        val settings = TuningSettings(scanWindowMs = 3_000L)
        val adjusted = settings.adjust(TuningField.ScanWindowMs, 1)
        assertEquals(3_500L, adjusted.scanWindowMs)

        val decreased = settings.adjust(TuningField.ScanWindowMs, -1)
        assertEquals(2_500L, decreased.scanWindowMs)
    }

    @Test
    fun scanWindowMs_adjustmentRespectsBounds() {
        val min = TuningSettings(scanWindowMs = 1_000L)
        val decremented = min.adjust(TuningField.ScanWindowMs, -1)
        assertEquals(1_000L, decremented.scanWindowMs)

        val max = TuningSettings(scanWindowMs = 10_000L)
        val incremented = max.adjust(TuningField.ScanWindowMs, 1)
        assertEquals(10_000L, incremented.scanWindowMs)
    }
}

class AppUiStateHelpersTest {
    @Test
    fun isJoinInProgress_falseWhenIdle() {
        val state = AppUiState(listenerState = ListenerLifecycleState.IDLE)
        assertFalse(state.isJoinInProgress())
    }

    @Test
    fun isJoinInProgress_falseWhenScanning() {
        val state = AppUiState(listenerState = ListenerLifecycleState.SCANNING)
        assertFalse(state.isJoinInProgress())
    }

    @Test
    fun isJoinInProgress_falseWhenSessionSelected() {
        val state = AppUiState(listenerState = ListenerLifecycleState.SESSION_SELECTED)
        assertFalse(state.isJoinInProgress())
    }

    @Test
    fun isJoinInProgress_falseWhenDisconnected() {
        val state = AppUiState(listenerState = ListenerLifecycleState.DISCONNECTED)
        assertFalse(state.isJoinInProgress())
    }

    @Test
    fun isJoinInProgress_falseWhenError() {
        val state = AppUiState(listenerState = ListenerLifecycleState.ERROR)
        assertFalse(state.isJoinInProgress())
    }

    @Test
    fun isJoinInProgress_trueForJoinRequestedState() {
        val state = AppUiState(listenerState = ListenerLifecycleState.JOIN_REQUESTED)
        assertTrue(state.isJoinInProgress())
    }

    @Test
    fun isJoinInProgress_trueForAwaitingApprovalState() {
        val state = AppUiState(listenerState = ListenerLifecycleState.AWAITING_APPROVAL)
        assertTrue(state.isJoinInProgress())
    }

    @Test
    fun isJoinInProgress_trueForApprovedState() {
        val state = AppUiState(listenerState = ListenerLifecycleState.APPROVED)
        assertTrue(state.isJoinInProgress())
    }

    @Test
    fun isJoinInProgress_trueForConnectingState() {
        val state = AppUiState(listenerState = ListenerLifecycleState.CONNECTING)
        assertTrue(state.isJoinInProgress())
    }

    @Test
    fun isJoinInProgress_trueForSyncingClockState() {
        val state = AppUiState(listenerState = ListenerLifecycleState.SYNCING_CLOCK)
        assertTrue(state.isJoinInProgress())
    }

    @Test
    fun isJoinInProgress_trueForBufferingState() {
        val state = AppUiState(listenerState = ListenerLifecycleState.BUFFERING)
        assertTrue(state.isJoinInProgress())
    }

    @Test
    fun isJoinInProgress_trueForPlayingState() {
        val state = AppUiState(listenerState = ListenerLifecycleState.PLAYING)
        assertTrue(state.isJoinInProgress())
    }

    @Test
    fun isJoinInProgress_trueForReconnectingState() {
        val state = AppUiState(listenerState = ListenerLifecycleState.RECONNECTING)
        assertTrue(state.isJoinInProgress())
    }

    @Test
    fun isJoinInProgress_trueForDesyncedState() {
        val state = AppUiState(listenerState = ListenerLifecycleState.DESYNCED)
        assertTrue(state.isJoinInProgress())
    }

    @Test
    fun canSelectSession_trueWhenNoJoinInProgress() {
        val state = AppUiState(listenerState = ListenerLifecycleState.IDLE)
        val session = SessionInfo("test-id", "Test", "Host", ApprovalMode.MANUAL, inviteCodeRequired = false)
        assertTrue(state.canSelectSession(session))
    }

    @Test
    fun canSelectSession_falseForDifferentSessionWhenJoinInProgress() {
        val selected = SessionInfo("a", "A", "Host", ApprovalMode.MANUAL, inviteCodeRequired = false)
        val other = SessionInfo("b", "B", "Host", ApprovalMode.MANUAL, inviteCodeRequired = false)
        val state = AppUiState(
            listenerState = ListenerLifecycleState.CONNECTING,
            selectedSession = selected,
        )
        assertFalse(state.canSelectSession(other))
    }

    @Test
    fun canSelectSession_trueForSameSessionWhenJoinInProgress() {
        val selected = SessionInfo("a", "A", "Host", ApprovalMode.MANUAL, inviteCodeRequired = false)
        val state = AppUiState(
            listenerState = ListenerLifecycleState.CONNECTING,
            selectedSession = selected,
        )
        assertTrue(state.canSelectSession(selected))
    }

    @Test
    fun canManualResync_falseWhenNoSelectedSession() {
        val state = AppUiState(selectedSession = null)
        assertFalse(state.canManualResync())
    }

    @Test
    fun canManualResync_falseWhenIdle() {
        val session = SessionInfo("test", "Test", "Host", ApprovalMode.MANUAL, inviteCodeRequired = false)
        val state = AppUiState(
            selectedSession = session,
            listenerState = ListenerLifecycleState.IDLE,
        )
        assertFalse(state.canManualResync())
    }

    @Test
    fun canManualResync_falseWhenScanning() {
        val session = SessionInfo("test", "Test", "Host", ApprovalMode.MANUAL, inviteCodeRequired = false)
        val state = AppUiState(
            selectedSession = session,
            listenerState = ListenerLifecycleState.SCANNING,
        )
        assertFalse(state.canManualResync())
    }

    @Test
    fun canManualResync_falseWhenSessionSelected() {
        val session = SessionInfo("test", "Test", "Host", ApprovalMode.MANUAL, inviteCodeRequired = false)
        val state = AppUiState(
            selectedSession = session,
            listenerState = ListenerLifecycleState.SESSION_SELECTED,
        )
        assertFalse(state.canManualResync())
    }

    @Test
    fun canManualResync_falseWhenDisconnected() {
        val session = SessionInfo("test", "Test", "Host", ApprovalMode.MANUAL, inviteCodeRequired = false)
        val state = AppUiState(
            selectedSession = session,
            listenerState = ListenerLifecycleState.DISCONNECTED,
        )
        assertFalse(state.canManualResync())
    }

    @Test
    fun canManualResync_falseWhenError() {
        val session = SessionInfo("test", "Test", "Host", ApprovalMode.MANUAL, inviteCodeRequired = false)
        val state = AppUiState(
            selectedSession = session,
            listenerState = ListenerLifecycleState.ERROR,
        )
        assertFalse(state.canManualResync())
    }

    @Test
    fun canManualResync_trueWhenSyncingClock() {
        val session = SessionInfo("test", "Test", "Host", ApprovalMode.MANUAL, inviteCodeRequired = false)
        val state = AppUiState(
            selectedSession = session,
            listenerState = ListenerLifecycleState.SYNCING_CLOCK,
        )
        assertTrue(state.canManualResync())
    }

    @Test
    fun canManualResync_trueWhenBuffering() {
        val session = SessionInfo("test", "Test", "Host", ApprovalMode.MANUAL, inviteCodeRequired = false)
        val state = AppUiState(
            selectedSession = session,
            listenerState = ListenerLifecycleState.BUFFERING,
        )
        assertTrue(state.canManualResync())
    }

    @Test
    fun canManualResync_trueWhenPlaying() {
        val session = SessionInfo("test", "Test", "Host", ApprovalMode.MANUAL, inviteCodeRequired = false)
        val state = AppUiState(
            selectedSession = session,
            listenerState = ListenerLifecycleState.PLAYING,
        )
        assertTrue(state.canManualResync())
    }
}

class BufferingStepTest {
    @Test
    fun bufferingStep_pendingBeforeSync() {
        val p = ConnectionProgressState(synced = false, buffered = false)
        assertEquals(StepState.Pending, p.bufferingStep())
    }

    @Test
    fun bufferingStep_activeAfterSyncBeforeBufferReady() {
        val p = ConnectionProgressState(synced = true, buffered = false)
        assertEquals(StepState.Active, p.bufferingStep())
    }

    @Test
    fun bufferingStep_doneWhenBuffered() {
        val p = ConnectionProgressState(buffered = true)
        assertEquals(StepState.Done, p.bufferingStep())
    }

    @Test
    fun playingStep_activeOnlyAfterBuffering() {
        val p = ConnectionProgressState(buffered = true, playing = false)
        assertEquals(StepState.Active, p.playingStep())
    }

    @Test
    fun playingStep_pendingBeforeBuffering() {
        val p = ConnectionProgressState(buffered = false, playing = false)
        assertEquals(StepState.Pending, p.playingStep())
    }

    @Test
    fun playingStep_doneWhenPlaying() {
        val p = ConnectionProgressState(buffered = true, playing = true)
        assertEquals(StepState.Done, p.playingStep())
    }
}
