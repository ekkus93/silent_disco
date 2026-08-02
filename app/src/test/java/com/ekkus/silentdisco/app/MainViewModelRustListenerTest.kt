package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.uniffi.FfiApprovalMode
import com.ekkus.silentdisco.core.uniffi.FfiListenerLifecycle
import com.ekkus.silentdisco.core.uniffi.FfiSessionAdvertisement
import org.junit.Assert.assertEquals
import org.junit.Test

class NextListenerStateTest {
    @Test
    fun rustDrivenTransitionsAreAlwaysAccepted() {
        assertEquals(
            ListenerLifecycleState.SCANNING,
            nextListenerState(ListenerLifecycleState.IDLE, ListenerLifecycleState.SCANNING),
        )
        assertEquals(
            ListenerLifecycleState.APPROVED,
            nextListenerState(ListenerLifecycleState.AWAITING_APPROVAL, ListenerLifecycleState.APPROVED),
        )
    }

    @Test
    fun playbackTailStateSurvivesAStaleRustEchoOfApproved() {
        // Rust doesn't drive PLAYING; once Kotlin's own playback pipeline has
        // moved there, a repeated snapshot still reporting APPROVED must not
        // silently regress the UI.
        assertEquals(
            ListenerLifecycleState.PLAYING,
            nextListenerState(ListenerLifecycleState.PLAYING, ListenerLifecycleState.APPROVED),
        )
        assertEquals(
            ListenerLifecycleState.BUFFERING,
            nextListenerState(ListenerLifecycleState.BUFFERING, ListenerLifecycleState.CONNECTING),
        )
    }

    @Test
    fun playbackTailStateCanAdvanceToAnotherPlaybackTailState() {
        assertEquals(
            ListenerLifecycleState.DESYNCED,
            nextListenerState(ListenerLifecycleState.PLAYING, ListenerLifecycleState.DESYNCED),
        )
        assertEquals(
            ListenerLifecycleState.RECONNECTING,
            nextListenerState(ListenerLifecycleState.DESYNCED, ListenerLifecycleState.RECONNECTING),
        )
    }

    @Test
    fun disconnectedAndErrorAlwaysOverridePlaybackTailState() {
        assertEquals(
            ListenerLifecycleState.DISCONNECTED,
            nextListenerState(ListenerLifecycleState.PLAYING, ListenerLifecycleState.DISCONNECTED),
        )
        assertEquals(
            ListenerLifecycleState.ERROR,
            nextListenerState(ListenerLifecycleState.BUFFERING, ListenerLifecycleState.ERROR),
        )
    }
}

class FfiListenerLifecycleMappingTest {
    @Test
    fun everyFfiListenerLifecycleMapsToItsNamedAppCounterpart() {
        val expected = mapOf(
            FfiListenerLifecycle.IDLE to ListenerLifecycleState.IDLE,
            FfiListenerLifecycle.SCANNING to ListenerLifecycleState.SCANNING,
            FfiListenerLifecycle.SESSION_SELECTED to ListenerLifecycleState.SESSION_SELECTED,
            FfiListenerLifecycle.JOIN_REQUESTED to ListenerLifecycleState.JOIN_REQUESTED,
            FfiListenerLifecycle.AWAITING_APPROVAL to ListenerLifecycleState.AWAITING_APPROVAL,
            FfiListenerLifecycle.APPROVED to ListenerLifecycleState.APPROVED,
            FfiListenerLifecycle.CONNECTING to ListenerLifecycleState.CONNECTING,
            FfiListenerLifecycle.SYNCING_CLOCK to ListenerLifecycleState.SYNCING_CLOCK,
            FfiListenerLifecycle.BUFFERING to ListenerLifecycleState.BUFFERING,
            FfiListenerLifecycle.PLAYING to ListenerLifecycleState.PLAYING,
            FfiListenerLifecycle.RECONNECTING to ListenerLifecycleState.RECONNECTING,
            FfiListenerLifecycle.DESYNCED to ListenerLifecycleState.DESYNCED,
            FfiListenerLifecycle.DISCONNECTED to ListenerLifecycleState.DISCONNECTED,
            FfiListenerLifecycle.ERROR to ListenerLifecycleState.ERROR,
        )
        expected.forEach { (ffi, app) ->
            assertEquals("$ffi should map to $app", app, ffi.toAppListenerLifecycle())
        }
    }
}

class SessionAdvertisementMappingTest {
    @Test
    fun ffiSessionAdvertisementMapsToAnAppSessionInfo() {
        val advertisement = FfiSessionAdvertisement(
            sessionId = "session-1",
            hostDeviceId = "host-1",
            sessionName = "Patio Mix",
            approvalMode = FfiApprovalMode.INVITE_CODE,
            protocolVersion = 2u,
            address = null,
            controlPort = null,
            syncPort = null,
            audioPort = null,
        )
        val session = advertisement.toAppSessionInfo()
        assertEquals("session-1", session.id)
        assertEquals("Patio Mix", session.name)
        assertEquals("host-1", session.hostDeviceName)
        assertEquals(ApprovalMode.INVITE_CODE, session.approvalMode)
        assertEquals(true, session.inviteCodeRequired)
    }

    @Test
    fun discoveredSessionWithoutAKnownEndpointRoundTripsWithoutOne() {
        val session = SessionInfo(
            id = "wifi-direct-session",
            name = "Nearby session",
            hostDeviceName = "host-device",
            approvalMode = ApprovalMode.MANUAL,
            inviteCodeRequired = false,
        )
        val advertisement = session.toFfiSessionAdvertisement()
        assertEquals("wifi-direct-session", advertisement.sessionId)
        assertEquals(null, advertisement.address)
        assertEquals(null, advertisement.controlPort)
        assertEquals(null, advertisement.syncPort)
        assertEquals(null, advertisement.audioPort)
    }
}
