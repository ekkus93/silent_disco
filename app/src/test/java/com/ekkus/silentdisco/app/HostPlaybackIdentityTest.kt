package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.transport.SendAllResult
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class HostPlaybackIdentityTest {

    @Test
    fun requireHostSession_returnsErrorWhenSessionNull() {
        val error = requireHostSessionForPlayback(currentSessionId = null)
        assertNotNull(error)
        assertEquals("Start a host session before starting playback", error)
    }

    @Test
    fun requireHostSession_returnsNullWhenSessionPresent() {
        val error = requireHostSessionForPlayback(currentSessionId = SessionId("session-abc"))
        assertNull(error)
    }

    @Test
    fun zeroPeerApproval_isNotSuccess() {
        val result = SendAllResult(peerCount = 0, successCount = 0, failureCount = 0)
        assertFalse(result.deliveredToAnyPeer)
        assertFalse(result.allDelivered)
    }

    @Test
    fun fullDelivery_isSuccess() {
        val result = SendAllResult(peerCount = 2, successCount = 2, failureCount = 0)
        assertTrue(result.deliveredToAnyPeer)
        assertTrue(result.allDelivered)
    }

    @Test
    fun partialDelivery_isNotAllDelivered() {
        val result = SendAllResult(peerCount = 2, successCount = 1, failureCount = 1)
        assertTrue(result.deliveredToAnyPeer)
        assertFalse(result.allDelivered)
    }
}
