package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import org.junit.Assert.assertEquals
import org.junit.Test

class ConnectionProgressStepTest {

    // --- discoveredStep ---

    @Test
    fun discoveredStep_notDiscovered_isActive() {
        val p = ConnectionProgressState(discovered = false)
        assertEquals(StepState.Active, p.discoveredStep())
    }

    @Test
    fun discoveredStep_discovered_isDone() {
        val p = ConnectionProgressState(discovered = true)
        assertEquals(StepState.Done, p.discoveredStep())
    }

    // --- requestedStep ---

    @Test
    fun requestedStep_nothingDone_isPending() {
        val p = ConnectionProgressState(discovered = false, requested = false)
        assertEquals(StepState.Pending, p.requestedStep())
    }

    @Test
    fun requestedStep_discoveredOnly_isActive() {
        val p = ConnectionProgressState(discovered = true, requested = false)
        assertEquals(StepState.Active, p.requestedStep())
    }

    @Test
    fun requestedStep_requested_isDone() {
        val p = ConnectionProgressState(discovered = true, requested = true)
        assertEquals(StepState.Done, p.requestedStep())
    }

    // --- approvedStep ---

    @Test
    fun approvedStep_nothingDone_isPending() {
        val p = ConnectionProgressState(requested = false, approved = false)
        assertEquals(StepState.Pending, p.approvedStep())
    }

    @Test
    fun approvedStep_requestedOnly_isActive() {
        val p = ConnectionProgressState(requested = true, approved = false)
        assertEquals(StepState.Active, p.approvedStep())
    }

    @Test
    fun approvedStep_approved_isDone() {
        val p = ConnectionProgressState(approved = true)
        assertEquals(StepState.Done, p.approvedStep())
    }

    // --- connectedStep ---

    @Test
    fun connectedStep_approvedOnly_isActive() {
        val p = ConnectionProgressState(approved = true, connected = false)
        assertEquals(StepState.Active, p.connectedStep())
    }

    @Test
    fun connectedStep_connected_isDone() {
        val p = ConnectionProgressState(connected = true)
        assertEquals(StepState.Done, p.connectedStep())
    }

    @Test
    fun connectedStep_notApproved_isPending() {
        val p = ConnectionProgressState(approved = false, connected = false)
        assertEquals(StepState.Pending, p.connectedStep())
    }

    // --- syncedStep ---

    @Test
    fun syncedStep_connectedOnly_isActive() {
        val p = ConnectionProgressState(connected = true, synced = false)
        assertEquals(StepState.Active, p.syncedStep())
    }

    @Test
    fun syncedStep_synced_isDone() {
        val p = ConnectionProgressState(synced = true)
        assertEquals(StepState.Done, p.syncedStep())
    }

    @Test
    fun syncedStep_notConnected_isPending() {
        val p = ConnectionProgressState(connected = false, synced = false)
        assertEquals(StepState.Pending, p.syncedStep())
    }

    // --- playingStep ---

    @Test
    fun playingStep_bufferedOnly_isActive() {
        val p = ConnectionProgressState(buffered = true, playing = false)
        assertEquals(StepState.Active, p.playingStep())
    }

    @Test
    fun playingStep_playing_isDone() {
        val p = ConnectionProgressState(playing = true)
        assertEquals(StepState.Done, p.playingStep())
    }

    @Test
    fun playingStep_notBuffered_isPending() {
        val p = ConnectionProgressState(buffered = false, playing = false)
        assertEquals(StepState.Pending, p.playingStep())
    }

    // --- full happy-path progression ---

    @Test
    fun allSteps_atIdleState_onlyDiscoveredIsActive() {
        val p = ConnectionProgressState()
        assertEquals(StepState.Active, p.discoveredStep())
        assertEquals(StepState.Pending, p.requestedStep())
        assertEquals(StepState.Pending, p.approvedStep())
        assertEquals(StepState.Pending, p.connectedStep())
        assertEquals(StepState.Pending, p.syncedStep())
        assertEquals(StepState.Pending, p.playingStep())
    }

    @Test
    fun allSteps_fullyPlaying_allAreDone() {
        val p = ConnectionProgressState(
            discovered = true,
            requested = true,
            approved = true,
            connected = true,
            synced = true,
            playing = true,
        )
        assertEquals(StepState.Done, p.discoveredStep())
        assertEquals(StepState.Done, p.requestedStep())
        assertEquals(StepState.Done, p.approvedStep())
        assertEquals(StepState.Done, p.connectedStep())
        assertEquals(StepState.Done, p.syncedStep())
        assertEquals(StepState.Done, p.playingStep())
    }
}
