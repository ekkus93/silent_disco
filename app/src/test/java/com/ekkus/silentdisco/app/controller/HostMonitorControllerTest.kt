package com.ekkus.silentdisco.app.controller

import com.ekkus.silentdisco.core.audio.AudioFormatSpec
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackConfig
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Assert.assertThrows
import org.junit.Test
import org.mockito.Mockito.inOrder
import org.mockito.Mockito.mock
import org.mockito.Mockito.verify
import org.mockito.Mockito.`when`

class HostMonitorControllerTest {
    private val parent = CoroutineScope(SupervisorJob())
    private val events = mutableListOf<String>()
    private val configs = mutableListOf<FfiListenerPlaybackConfig>()
    private val runtimes = ArrayDeque<FfiListenerPlaybackHandle>()
    private var outputStatus = 0

    @After
    fun cancelParent() = parent.cancel()

    private fun runtime(token: Long): FfiListenerPlaybackHandle =
        mock(FfiListenerPlaybackHandle::class.java).also { `when`(it.engineToken()).thenReturn(token) }

    private fun controller() = HostMonitorController(
        parentScope = parent,
        closeOutput = { events += "closeOutput" },
        openOutput = { token ->
            events += "openOutput:$token"
            outputStatus
        },
        openRuntime = { config ->
            configs += config
            runtimes.removeFirst()
        },
        elapsedRealtimeMs = { 1_234L },
    )

    private val session = SessionId("session-1")
    private val stream = StreamId("stream-1")
    private val format = AudioFormatSpec(sampleRate = 48_000, channelCount = 2)

    private fun packet(sessionId: SessionId = session, streamId: StreamId = stream) = AudioPacket(
        version = 3,
        codec = "pcm_s16le",
        sessionId = sessionId,
        streamId = streamId,
        sequenceNumber = 0,
        sampleRate = 48_000,
        channelCount = 2,
        samplesPerPacket = 240,
        firstSampleIndex = 0,
        hostPresentationTimeMs = 5_000L,
        payload = ByteArray(960),
    )

    @Test
    fun startOpensTheRuntimeLocksTheHostClockAndOpensOutput() {
        val first = runtime(token = 77L)
        runtimes += first
        val monitor = controller()

        monitor.start(session, stream, format, packet(), volume = 0.5f)

        assertThat(monitor.runtime).isSameInstanceAs(first)
        assertThat(configs.single().hostStartTimeMs).isEqualTo(5_000uL)
        assertThat(configs.single().volume).isEqualTo(0.5f)
        verify(first).lockSameProcessHostClock(1_234uL)
        assertThat(events).containsExactly("openOutput:77")
    }

    @Test
    fun aFailedOutputOpenCleansUpInOrderAndLeavesNothingRunning() {
        val failing = runtime(token = 9L)
        runtimes += failing
        outputStatus = -1
        val monitor = controller()

        val error = assertThrows(IllegalStateException::class.java) {
            monitor.start(session, stream, format, packet(), volume = 1f)
        }

        assertThat(error.message).contains("status=-1")
        val order = inOrder(failing)
        order.verify(failing).stop()
        order.verify(failing).close()
        assertThat(events).containsExactly("openOutput:9", "closeOutput").inOrder()
        assertThat(monitor.runtime).isNull()
    }

    @Test
    fun startingAgainStopsTheRunningMonitorFirst() {
        val first = runtime(token = 1L)
        val second = runtime(token = 2L)
        runtimes += first
        runtimes += second
        val monitor = controller()
        monitor.start(session, stream, format, packet(), volume = 1f)

        monitor.start(session, stream, format, packet(), volume = 1f)

        verify(first).stop()
        verify(first).close()
        assertThat(monitor.runtime).isSameInstanceAs(second)
        assertThat(events).containsExactly("openOutput:1", "closeOutput", "openOutput:2").inOrder()
    }

    @Test
    fun startRejectsAPacketFromAnotherStreamOrSessionAndSubmitNeedsARunningMonitor() {
        val monitor = controller()
        assertThrows(IllegalArgumentException::class.java) {
            monitor.start(session, stream, format, packet(streamId = StreamId("other")), volume = 1f)
        }
        assertThrows(IllegalArgumentException::class.java) {
            monitor.start(session, stream, format, packet(sessionId = SessionId("other")), volume = 1f)
        }
        assertThat(configs).isEmpty()
        assertThrows(IllegalStateException::class.java) { monitor.submit(packet()) }
        assertThrows(IllegalArgumentException::class.java) { monitor.reanchor(-1L) }
    }
}
