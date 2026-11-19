package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import com.ekkus.silentdisco.core.uniffi.FfiListenerSyncHandover
import com.ekkus.silentdisco.core.uniffi.FfiListenerSyncSession
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportHandle
import com.ekkus.silentdisco.core.uniffi.FfiSyncConfidence
import com.ekkus.silentdisco.core.uniffi.FfiSyncSampleOutcome
import com.google.common.truth.Truth.assertThat
import org.junit.Test
import org.mockito.Mockito.mock
import org.mockito.Mockito.never
import org.mockito.Mockito.times
import org.mockito.Mockito.verify
import org.mockito.Mockito.`when`

/**
 * The manual listener's clock lock before its first stream (startup-counter
 * investigation, 2026-11-19): approval opens a Rust sync session, probes and
 * responses go to it until a stream runtime exists, and a teardown releases
 * it. Whether a stream opened from its handover starts locked is Rust's,
 * tested in `sync_session_tests.rs` and on the device.
 */
class ManualPreStreamSyncTest {
    private val outcome = FfiSyncSampleOutcome(
        accepted = true,
        offsetMs = 491_000.0,
        skewPpm = 0.0,
        roundTripTimeMs = 20.0,
        jitterMs = 0.0,
        confidence = FfiSyncConfidence.GOOD,
        acceptedSampleCount = 1uL,
        syncLocked = true,
        acquisitionRejectedSampleCount = 0uL,
        acquisitionElapsedMs = 20uL,
        acquisitionRttLimitMs = 200.0,
        degradedLock = false,
    )
    private val sessions = mutableListOf<FfiListenerSyncSession>()
    private val controller = ManualListenerTransportController(
        closeNativeOutput = {},
        newSyncSession = {
            mock(FfiListenerSyncSession::class.java).also { session ->
                `when`(session.observeSyncResponse(1uL, 100uL, 500_000uL, 500_002uL, 120uL)).thenReturn(outcome)
                sessions += session
            }
        },
    )
    private val handle = mock(FfiListenerTransportHandle::class.java)

    init {
        controller.handleGate.replace(retire = {}, open = { handle }, start = {})
    }

    private fun syncResponse() = FfiListenerTransportEvent.SyncResponseReceived(
        correlationId = 1uL,
        t1ListenerSendElapsedMs = 100uL,
        t2HostReceiveElapsedMs = 500_000uL,
        t3HostSendElapsedMs = 500_002uL,
        receivedAtElapsedMs = 9_120uL,
    )

    @Test
    fun approvalOpensOneSessionAndAnchorsTheTransportClockToIt() {
        `when`(handle.nowMs()).thenReturn(9_000uL)
        controller.openPreStreamSync(handle)
        val session = sessions.single()
        assertThat(controller.preStreamSync).isSameInstanceAs(session)
        // The session clock read 0 when the transport read 9 000.
        assertThat(controller.transportClockOriginMs).isEqualTo(9_000uL)

        controller.openPreStreamSync(handle)
        assertThat(sessions).hasSize(1)
    }

    @Test
    fun noSessionOpensWhenAStreamOrACarriedLockAlreadyExists() {
        controller.playbackRuntime = mock(FfiListenerPlaybackHandle::class.java)
        controller.openPreStreamSync(handle)
        controller.playbackRuntime = null
        controller.syncHandover = mock(FfiListenerSyncHandover::class.java)
        controller.openPreStreamSync(handle)
        assertThat(sessions).isEmpty()
    }

    @Test
    fun responsesGoToTheSessionUntilAStreamRuntimeExists() {
        `when`(handle.nowMs()).thenReturn(9_000uL)
        controller.openPreStreamSync(handle)
        val session = sessions.single()

        controller.handleSyncResponse(handle, syncResponse())
        // t4 translated onto the session clock: 9 120 - 9 000.
        verify(session).observeSyncResponse(1uL, 100uL, 500_000uL, 500_002uL, 120uL)

        val runtime = mock(FfiListenerPlaybackHandle::class.java)
        `when`(runtime.observeSyncResponse(1uL, 100uL, 500_000uL, 500_002uL, 120uL)).thenReturn(outcome)
        controller.playbackRuntime = runtime
        controller.handleSyncResponse(handle, syncResponse())
        verify(runtime).observeSyncResponse(1uL, 100uL, 500_000uL, 500_002uL, 120uL)
        verify(session, times(1)).observeSyncResponse(1uL, 100uL, 500_000uL, 500_002uL, 120uL)
    }

    @Test
    fun aTeardownReleasesTheSessionAndAStreamChangeDoesNot() {
        controller.openPreStreamSync(handle)
        val session = sessions.single()

        controller.endStream(keepOutputOpen = true)
        verify(session, never()).close()
        assertThat(controller.preStreamSync).isSameInstanceAs(session)

        controller.stopPlayback()
        verify(session).close()
        assertThat(controller.preStreamSync).isNull()
    }

    @Test
    fun aSessionThatCannotOpenIsNotFatal() {
        val failing = ManualListenerTransportController(
            closeNativeOutput = {},
            newSyncSession = { throw IllegalStateException("estimator unavailable") },
        )
        failing.openPreStreamSync(handle)
        assertThat(failing.preStreamSync).isNull()
    }
}
