package com.ekkus.silentdisco.app.controller

import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.rust.RustSyncEstimator
import com.ekkus.silentdisco.core.rust.RustSyncObservation
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackConfig
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import com.ekkus.silentdisco.core.uniffi.FfiListenerSyncHandover
import com.google.common.truth.Truth.assertThat
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Test
import org.mockito.Mockito.inOrder
import org.mockito.Mockito.mock
import org.mockito.Mockito.never
import org.mockito.Mockito.times
import org.mockito.Mockito.verify
import org.mockito.Mockito.`when`

class ListenerPlaybackControllerTest {
    private val parent = CoroutineScope(SupervisorJob())
    private val events = mutableListOf<String>()
    private val opened = mutableListOf<Pair<FfiListenerPlaybackConfig, FfiListenerSyncHandover?>>()
    private val runtimes = ArrayDeque<FfiListenerPlaybackHandle>()
    private var outputStatus = 0
    private var rebindStatus = 0
    private var outputOpen = false
    private var estimatorsOpened = 0

    @After
    fun cancelParent() = parent.cancel()

    private fun runtime(token: Long, handover: FfiListenerSyncHandover = mock(FfiListenerSyncHandover::class.java)) =
        mock(FfiListenerPlaybackHandle::class.java).also {
            `when`(it.engineToken()).thenReturn(token)
            `when`(it.syncHandover()).thenReturn(handover)
        }

    private val estimator = mock(RustSyncEstimator::class.java)

    private fun controller() = ListenerPlaybackController(
        parentScope = parent,
        attachOutput = { events += "attach:${it.engineToken()}" },
        detachOutput = { events += "detach" },
        openOutput = { token ->
            events += "openOutput:$token"
            outputOpen = outputStatus == 0
            outputStatus
        },
        closeOutput = {
            events += "closeOutput"
            outputOpen = false
        },
        isOutputOpen = { outputOpen },
        rebindOutput = { token ->
            events += "rebind:$token"
            rebindStatus
        },
        openRuntime = { config, handover ->
            opened += config to handover
            runtimes.removeFirst()
        },
        openEstimator = {
            estimatorsOpened += 1
            estimator
        },
    )

    private fun start(session: String, hostStartTimeMs: ULong = 1_000uL) = ListenerStreamStart(
        sessionId = SessionId(session),
        streamId = StreamId("stream-$hostStartTimeMs"),
        sampleRate = 48_000u,
        hostStartTimeMs = hostStartTimeMs,
        samplesPerPacket = 960u,
        channels = 2u,
        startupBufferMs = 300L,
    )

    @Test
    fun aLaterStreamOfTheSameSessionOpensFromTheLastStreamsClockLock() {
        val lock = mock(FfiListenerSyncHandover::class.java)
        val first = runtime(token = 1L, handover = lock)
        val second = runtime(token = 2L)
        runtimes += first
        runtimes += second
        val playback = controller()

        assertThat(playback.startNow(start("a"), reanchorRunning = false))
            .isInstanceOf(ListenerStreamStartResult.Started::class.java)
        val result = playback.startNow(start("a", hostStartTimeMs = 9_000uL), reanchorRunning = false)

        assertThat(result).isEqualTo(ListenerStreamStartResult.Started(second, emptyList()))
        // The old stream ends where the new one begins on the host timeline.
        verify(first).stopAt(9_000uL)
        assertThat(opened.map { it.second }).containsExactly(null, lock).inOrder()
        assertThat(opened.last().first.startupBufferTargetMs).isEqualTo(300uL)
        verify(lock).close()
        // The output stays open and is rebound to the new stream (owner
        // decision 2026-11-18: a reopen can downgrade the device's stream).
        assertThat(events).containsExactly(
            "openOutput:1", "attach:1", "detach", "rebind:2", "attach:2",
        ).inOrder()
    }

    @Test
    fun aHostStopKeepsTheOutputOpenAndTheNextStreamRebindsIt() {
        runtimes += runtime(token = 1L)
        runtimes += runtime(token = 2L)
        val playback = controller()
        playback.startNow(start("a"), reanchorRunning = false)

        assertThat(playback.stopNow(cutAtHostMs = 5_000uL, keepOutputOpen = true)).isNull()
        assertThat(outputOpen).isTrue()
        playback.startNow(start("a", hostStartTimeMs = 9_000uL), reanchorRunning = false)

        assertThat(events).containsExactly(
            "openOutput:1", "attach:1", "detach", "rebind:2", "attach:2",
        ).inOrder()
    }

    @Test
    fun aFailedRebindFallsBackToAFreshOutput() {
        runtimes += runtime(token = 1L)
        runtimes += runtime(token = 2L)
        val playback = controller()
        playback.startNow(start("a"), reanchorRunning = false)
        rebindStatus = -7

        val result = playback.startNow(start("a", hostStartTimeMs = 9_000uL), reanchorRunning = false)

        assertThat(result).isInstanceOf(ListenerStreamStartResult.Started::class.java)
        assertThat(events.drop(2)).containsExactly(
            "detach", "rebind:2", "closeOutput", "openOutput:2", "attach:2",
        ).inOrder()
    }

    @Test
    fun aTeardownStillClosesTheOutput() {
        runtimes += runtime(token = 1L)
        val playback = controller()
        playback.startNow(start("a"), reanchorRunning = false)

        assertThat(playback.stopNow()).isNull()

        assertThat(events.takeLast(2)).containsExactly("detach", "closeOutput").inOrder()
        assertThat(outputOpen).isFalse()
    }

    @Test
    fun anotherSessionsClockLockIsClosedNotReused() {
        val lock = mock(FfiListenerSyncHandover::class.java)
        runtimes += runtime(token = 1L, handover = lock)
        runtimes += runtime(token = 2L)
        val playback = controller()

        playback.startNow(start("a"), reanchorRunning = false)
        playback.startNow(start("b"), reanchorRunning = false)

        assertThat(opened.last().second).isNull()
        verify(lock).close()
    }

    @Test
    fun aResumeReanchorsTheRunningRuntimeInPlace() {
        val first = runtime(token = 1L)
        runtimes += first
        val playback = controller()
        playback.startNow(start("a"), reanchorRunning = false)

        val result = playback.startNow(start("a", hostStartTimeMs = 4_000uL), reanchorRunning = true)

        assertThat(result).isEqualTo(ListenerStreamStartResult.Reanchored(null))
        verify(first).reanchorPresentationTime(4_000uL)
        verify(first, never()).stop()
        assertThat(playback.runtime).isSameInstanceAs(first)
    }

    @Test
    fun aFailedOutputOpenStopsTheNewRuntimeAndKeepsItsLock() {
        val lock = mock(FfiListenerSyncHandover::class.java)
        val failing = runtime(token = 5L, handover = lock)
        runtimes += failing
        outputStatus = -3
        val playback = controller()

        val result = playback.startNow(start("a"), reanchorRunning = false)

        result as ListenerStreamStartResult.Failed
        assertThat(result.error).hasMessageThat().contains("status=-3")
        assertThat(result.stopFailures).isEmpty()
        val order = inOrder(failing)
        order.verify(failing).stop()
        order.verify(failing).syncHandover()
        order.verify(failing).close()
        assertThat(events).containsExactly("openOutput:5", "detach", "closeOutput").inOrder()
        assertThat(playback.runtime).isNull()
        assertThat(playback.syncHandover?.handover).isSameInstanceAs(lock)
    }

    @Test
    fun stopAttemptsEveryStepAndReturnsTheFirstFailure() {
        val running = runtime(token = 1L)
        runtimes += running
        val playback = controller()
        playback.startNow(start("a"), reanchorRunning = false)
        val stopFailure = IllegalStateException("stop failed")
        `when`(running.stop()).thenThrow(stopFailure)

        assertThat(playback.stopNow()).isSameInstanceAs(stopFailure)

        verify(running).close()
        assertThat(events.takeLast(2)).containsExactly("detach", "closeOutput").inOrder()
        assertThat(playback.stopNow()).isNull()
    }

    @Test
    fun theEstimatorOpensOnceAndClosesOnRelease() {
        `when`(estimator.observe(1L, 2L, 3L, 4L)).thenReturn(mock(RustSyncObservation::class.java))
        val playback = controller()

        playback.observeSync(1L, 2L, 3L, 4L, maxSamples = 8)
        playback.observeSync(1L, 2L, 3L, 4L, maxSamples = 8)
        assertThat(estimatorsOpened).isEqualTo(1)
        verify(estimator, times(2)).observe(1L, 2L, 3L, 4L)

        playback.close()
        playback.close()

        verify(estimator).close()
        assertThat(playback.syncEstimator).isNull()
        assertThat(playback.executor.awaitTermination(5, TimeUnit.SECONDS)).isTrue()
    }

    @Test
    fun releaseClosesTheStoredClockLockAfterQueuedWork() {
        val lock = mock(FfiListenerSyncHandover::class.java)
        runtimes += runtime(token = 1L, handover = lock)
        val playback = controller()
        playback.startNow(start("a"), reanchorRunning = false)
        // A stop already queued on the control thread captures the lock first.
        playback.execute { playback.stopNow() }

        playback.close()

        assertThat(playback.executor.awaitTermination(5, TimeUnit.SECONDS)).isTrue()
        verify(lock).close()
        assertThat(playback.syncHandover).isNull()
    }
}
