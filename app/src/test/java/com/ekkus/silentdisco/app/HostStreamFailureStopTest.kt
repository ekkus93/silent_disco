package com.ekkus.silentdisco.app

import com.google.common.truth.Truth.assertThat
import com.ekkus.silentdisco.core.audio.AudioFormatSpec
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.rust.HostPlaylistStreamController
import com.ekkus.silentdisco.core.rust.HostTransportController
import com.ekkus.silentdisco.core.uniffi.FfiAudioPacket
import com.ekkus.silentdisco.core.uniffi.FfiHostPlaylistException
import com.ekkus.silentdisco.core.uniffi.FfiHostPlaylistStreamer
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportDelivery
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportHandle
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistFrame
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistStreamStart
import com.ekkus.silentdisco.core.uniffi.NoHandle
import java.util.Collections
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Before
import org.junit.Test

/**
 * Drives the production Android host stream loop (`startHostStreamingLoop`)
 * with virtual monotonic time and recording native-handle fakes, so the
 * broadcast Stop for an exceptional late decode failure is observed with
 * the timestamp listeners actually receive.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class HostStreamFailureStopTest {
    private val dispatcher = StandardTestDispatcher()

    @Before
    fun setUpMainDispatcher() {
        Dispatchers.setMain(dispatcher)
    }

    @After
    fun resetMainDispatcher() {
        Dispatchers.resetMain()
    }

    @Test
    fun lateDecodeFailureBroadcastsStopOnlyAfterTheLastSentPacketIsAudible() = runTest(dispatcher) {
        val fixture = startFailingStream(
            frames = listOf(audioFrame(sequence = 1, firstSampleIndex = 960, presentationMs = 420)),
        )
        try {
            // Packets go out at 0 and 20 ms; the decoder fails at 40 ms. The
            // last packet (presented at 420 ms) is audible until 440 ms.
            advanceTimeBy(439L)
            runCurrent()
            assertThat(fixture.transport.audio.map { it.sequence to it.sentAtMs })
                .containsExactly(0uL to 0L, 1uL to 20L)
                .inOrder()
            assertThat(fixture.transport.stops).isEmpty()
            assertThat(fixture.harness.hostCoreController.playbackFailureCalls).isEmpty()

            advanceTimeBy(1L)
            runCurrent()
            assertThat(fixture.transport.stops)
                .containsExactly(StopCall(STREAM_ID, hostStopTimeMs = 440uL, sentAtMs = 440L))
            assertThat(fixture.harness.hostCoreController.playbackFailureCalls)
                .containsExactly(DECODE_FAILURE)
            assertThat(fixture.viewModel.uiState.value.lastError).isEqualTo(DECODE_FAILURE)
            assertThat(fixture.monitor.stopped).isTrue()
            assertThat(fixture.streamer.cancelled).isTrue()
            assertThat(fixture.viewModel.hostPlaylistStreamer).isNull()
        } finally {
            fixture.viewModel.hostTransportController.close()
        }
    }

    @Test
    fun lateDecodeFailureAfterPauseTimestampsStopAfterThePauseShiftedTail() = runTest(dispatcher) {
        val fixture = startFailingStream(
            frames = listOf(
                audioFrame(sequence = 1, firstSampleIndex = 960, presentationMs = 420),
                audioFrame(sequence = 2, firstSampleIndex = 1_920, presentationMs = 440),
            ),
        )
        try {
            advanceTimeBy(30L)
            runCurrent()
            fixture.setPlaybackState(PlaybackState.PAUSED)
            advanceTimeBy(100L)
            runCurrent()
            fixture.setPlaybackState(PlaybackState.PLAYING)

            // The loop observed the pause at 40 ms and the resume at 140 ms,
            // so every later presentation time shifts by 100 ms. Packet 2
            // goes out at 140 ms for 540 ms; the decoder fails at 160 ms.
            advanceTimeBy(429L)
            runCurrent()
            assertThat(fixture.transport.audio.map { Triple(it.sequence, it.presentationMs, it.sentAtMs) })
                .containsExactly(
                    Triple(0uL, 400uL, 0L),
                    Triple(1uL, 420uL, 20L),
                    Triple(2uL, 540uL, 140L),
                )
                .inOrder()
            assertThat(fixture.transport.streamStartTimesMs).containsExactly(HOST_START_MS + 100uL)
            assertThat(fixture.monitor.reanchoredAtMs).containsExactly(520uL)
            assertThat(fixture.transport.stops).isEmpty()

            advanceTimeBy(1L)
            runCurrent()
            assertThat(fixture.transport.stops)
                .containsExactly(StopCall(STREAM_ID, hostStopTimeMs = 560uL, sentAtMs = 560L))
            assertThat(fixture.harness.hostCoreController.playbackFailureCalls)
                .containsExactly(DECODE_FAILURE)
        } finally {
            fixture.viewModel.hostTransportController.close()
        }
    }

    @Test
    fun failureBeforeAnyNextTrackAudioNeverAdvancesTheHostQueue() = runTest(dispatcher) {
        // The boundary into track two arrives, then the decoder fails before
        // any of track two's audio is sent: listeners never hear track two.
        val fixture = startFailingStream(frames = listOf(trackBoundary(firstSampleIndex = 960)))
        try {
            // The failure at 20 ms withdraws the unheard boundary at once,
            // not at 420 ms where its deadline ties with the tail's end.
            advanceTimeBy(20L)
            runCurrent()
            assertThat(fixture.viewModel.hostBoundaryJobs).isEmpty()
            assertThat(fixture.transport.stops).isEmpty()

            advanceTimeBy(980L)
            runCurrent()
            assertThat(fixture.harness.hostCoreController.trackBoundaryScheduledCalls)
                .containsExactly("entry-2" to 960L)
            assertThat(fixture.transport.boundarySamples).containsExactly(960uL)
            assertThat(fixture.transport.stops)
                .containsExactly(StopCall(STREAM_ID, hostStopTimeMs = 420uL, sentAtMs = 420L))
            assertThat(fixture.harness.hostCoreController.trackBoundaryAppliedCalls).isEmpty()
            assertThat(fixture.viewModel.hostBoundaryJobs).isEmpty()
        } finally {
            fixture.viewModel.hostTransportController.close()
        }
    }

    @Test
    fun failureAfterNextTrackAudioAppliesTheBoundaryBeforeTheStop() = runTest(dispatcher) {
        // Track two's first packet (sample 960, presented at 420 ms) is sent
        // before the decoder fails, so listeners hear the transition.
        val fixture = startFailingStream(
            frames = listOf(
                trackBoundary(firstSampleIndex = 960),
                audioFrame(sequence = 1, firstSampleIndex = 960, presentationMs = 420),
            ),
        )
        try {
            advanceTimeBy(419L)
            runCurrent()
            assertThat(fixture.harness.hostCoreController.trackBoundaryAppliedCalls).isEmpty()

            advanceTimeBy(1L)
            runCurrent()
            assertThat(fixture.harness.hostCoreController.trackBoundaryAppliedCalls)
                .containsExactly("entry-2" to 960L)
            assertThat(fixture.transport.stops).isEmpty()

            advanceTimeBy(20L)
            runCurrent()
            assertThat(fixture.transport.stops)
                .containsExactly(StopCall(STREAM_ID, hostStopTimeMs = 440uL, sentAtMs = 440L))
            assertThat(fixture.harness.hostCoreController.playbackFailureCalls)
                .containsExactly(DECODE_FAILURE)
        } finally {
            fixture.viewModel.hostTransportController.close()
        }
    }

    @Test
    fun twoTrackBoundariesRemainOrderedAcrossPauseAndResume() = runTest(dispatcher) {
        // Both tracks have audio scheduled on the same stream. Pause before
        // the second boundary is read: it must not apply against the old
        // unpaused clock, nor overtake the first boundary on resume.
        val fixture = startFailingStream(
            frames = listOf(
                trackBoundary(firstSampleIndex = 960),
                audioFrame(sequence = 1, firstSampleIndex = 960, presentationMs = 420),
                trackBoundary(firstSampleIndex = 1_920, entryId = "entry-3"),
                audioFrame(sequence = 2, firstSampleIndex = 1_920, presentationMs = 440),
            ),
        )
        try {
            advanceTimeBy(30L)
            runCurrent()
            fixture.setPlaybackState(PlaybackState.PAUSED)
            advanceTimeBy(100L)
            runCurrent()
            assertThat(fixture.harness.hostCoreController.trackBoundaryAppliedCalls).isEmpty()
            fixture.setPlaybackState(PlaybackState.PLAYING)

            advanceTimeBy(389L)
            runCurrent()
            assertThat(fixture.harness.hostCoreController.trackBoundaryScheduledCalls)
                .containsExactly("entry-2" to 960L, "entry-3" to 1_920L)
                .inOrder()
            assertThat(fixture.harness.hostCoreController.trackBoundaryAppliedCalls).isEmpty()
            assertThat(fixture.transport.stops).isEmpty()

            advanceTimeBy(1L)
            runCurrent()
            assertThat(fixture.harness.hostCoreController.trackBoundaryAppliedCalls)
                .containsExactly("entry-2" to 960L)
            assertThat(fixture.transport.stops).isEmpty()

            advanceTimeBy(20L)
            runCurrent()
            assertThat(fixture.harness.hostCoreController.trackBoundaryAppliedCalls)
                .containsExactly("entry-2" to 960L, "entry-3" to 1_920L)
                .inOrder()
            assertThat(fixture.transport.stops).isEmpty()

            advanceTimeBy(20L)
            runCurrent()
            assertThat(fixture.transport.stops)
                .containsExactly(StopCall(STREAM_ID, hostStopTimeMs = 560uL, sentAtMs = 560L))
        } finally {
            fixture.viewModel.hostTransportController.close()
        }
    }

    @Test
    fun stopDuringPauseCancelsAnAnnouncedButUnappliedBoundary() = runTest(dispatcher) {
        val fixture = startFailingStream(
            frames = listOf(
                trackBoundary(firstSampleIndex = 960),
                audioFrame(sequence = 1, firstSampleIndex = 960, presentationMs = 420),
            ),
        )
        try {
            advanceTimeBy(30L)
            runCurrent()
            assertThat(fixture.harness.hostCoreController.trackBoundaryScheduledCalls)
                .containsExactly("entry-2" to 960L)
            fixture.setPlaybackState(PlaybackState.PAUSED)
            advanceTimeBy(100L)
            runCurrent()
            assertThat(fixture.harness.hostCoreController.trackBoundaryAppliedCalls).isEmpty()

            // A user Stop during a pause must withdraw pending boundary jobs;
            // it cannot apply the old track change after the stream ends.
            fixture.setPlaybackState(PlaybackState.STOPPED)
            advanceTimeBy(500L)
            runCurrent()
            assertThat(fixture.viewModel.hostBoundaryJobs).isEmpty()
            assertThat(fixture.harness.hostCoreController.trackBoundaryAppliedCalls).isEmpty()
            assertThat(fixture.viewModel.hostPlaylistStreamer).isNull()
        } finally {
            fixture.viewModel.hostTransportController.close()
        }
    }

    @Test
    fun partialBoundaryDeliveryStopsBeforeSendingNextTrackAudio() = runTest(dispatcher) {
        val fixture = startFailingStream(
            frames = listOf(
                trackBoundary(firstSampleIndex = 960),
                audioFrame(sequence = 1, firstSampleIndex = 960, presentationMs = 420),
            ),
        )
        fixture.transport.boundaryDeliveryOverride = FfiHostTransportDelivery(
            intendedPeers = 2u,
            successfulPeers = 1u,
            failedPeers = 1u,
            bytesSent = 0uL,
        )
        try {
            advanceTimeBy(419L)
            runCurrent()
            assertThat(fixture.transport.boundarySamples).containsExactly(960uL)
            assertThat(fixture.transport.audio.map(AudioCall::sequence)).containsExactly(0uL)
            assertThat(fixture.harness.hostCoreController.trackBoundaryAppliedCalls).isEmpty()
            assertThat(fixture.transport.stops).isEmpty()

            advanceTimeBy(1L)
            runCurrent()
            assertThat(fixture.transport.stops)
                .containsExactly(StopCall(STREAM_ID, hostStopTimeMs = 420uL, sentAtMs = 420L))
            assertThat(fixture.harness.hostCoreController.playbackFailureCalls).hasSize(1)
            assertThat(fixture.harness.hostCoreController.playbackFailureCalls.single())
                .contains("Track boundary was not delivered")
        } finally {
            fixture.viewModel.hostTransportController.close()
        }
    }

    @Test
    fun boundaryTransportExceptionStopsWithoutSendingNextTrackAudio() = runTest(dispatcher) {
        val fixture = startFailingStream(
            frames = listOf(
                trackBoundary(firstSampleIndex = 960),
                audioFrame(sequence = 1, firstSampleIndex = 960, presentationMs = 420),
            ),
        )
        fixture.transport.boundaryException = IllegalStateException("injected boundary transport failure")
        try {
            advanceTimeBy(420L)
            runCurrent()
            assertThat(fixture.transport.audio.map(AudioCall::sequence)).containsExactly(0uL)
            assertThat(fixture.transport.stops)
                .containsExactly(StopCall(STREAM_ID, hostStopTimeMs = 420uL, sentAtMs = 420L))
            assertThat(fixture.harness.hostCoreController.trackBoundaryAppliedCalls).isEmpty()
            assertThat(fixture.harness.hostCoreController.playbackFailureCalls)
                .containsExactly("injected boundary transport failure")
        } finally {
            fixture.viewModel.hostTransportController.close()
        }
    }

    @Test
    fun boundaryWithNoConnectedListenersDoesNotAbortPlayback() = runTest(dispatcher) {
        val fixture = startFailingStream(
            frames = listOf(
                trackBoundary(firstSampleIndex = 960),
                audioFrame(sequence = 1, firstSampleIndex = 960, presentationMs = 420),
            ),
        )
        fixture.transport.boundaryDeliveryOverride = FfiHostTransportDelivery(
            intendedPeers = 0u,
            successfulPeers = 0u,
            failedPeers = 0u,
            bytesSent = 0uL,
        )
        try {
            advanceTimeBy(30L)
            runCurrent()
            assertThat(fixture.transport.audio.map(AudioCall::sequence))
                .containsExactly(0uL, 1uL).inOrder()
            assertThat(fixture.harness.hostCoreController.playbackFailureCalls).isEmpty()
            assertThat(fixture.transport.stops).isEmpty()
        } finally {
            fixture.viewModel.hostTransportController.close()
        }
    }

    private suspend fun TestScope.startFailingStream(frames: List<FfiPlaylistFrame>): StreamFixture {
        val clock = { testScheduler.currentTime }
        val transport = RecordingHostTransportHandle(clock)
        val harness = newTestMainViewModelHarness(
            hostTransportController = HostTransportController(callDispatcher = dispatcher) { _, _, _, _, _ ->
                transport
            },
            hostElapsedRealtimeMs = clock,
            hostStreamDispatcher = dispatcher,
        )
        val viewModel = harness.viewModel
        viewModel.hostTransportController.bind(backgroundScope, "127.0.0.1", 1, 2, 3, SESSION_ID)
        viewModel.ensureRustHostCore()
        val monitor = RecordingMonitorPlayback()
        val streamer = FailingPlaylistStreamer(frames)
        val controller = HostPlaylistStreamController(streamer)
        viewModel.hostMonitorPlayback = monitor
        viewModel.hostPlaylistStreamer = controller
        viewModel.currentStreamId = StreamId(STREAM_ID)
        viewModel._uiState.value = viewModel._uiState.value.copy(hostPlaybackState = PlaybackState.PLAYING)

        viewModel.startHostStreamingLoop(
            streamId = StreamId(STREAM_ID),
            streamer = controller,
            firstPacket = AudioPacket(
                version = 3,
                sessionId = SessionId(SESSION_ID),
                streamId = StreamId(STREAM_ID),
                sequenceNumber = 0,
                codec = "pcm16le",
                sampleRate = 48_000,
                channelCount = 2,
                samplesPerPacket = SAMPLES_PER_PACKET,
                firstSampleIndex = 0,
                hostPresentationTimeMs = HOST_START_MS.toLong(),
                payload = ByteArray(SAMPLES_PER_PACKET * 4),
            ),
            format = AudioFormatSpec(),
        )
        return StreamFixture(harness, transport, monitor, streamer)
    }

    private class StreamFixture(
        val harness: MainViewModelHarness,
        val transport: RecordingHostTransportHandle,
        val monitor: RecordingMonitorPlayback,
        val streamer: FailingPlaylistStreamer,
    ) {
        val viewModel: MainViewModel get() = harness.viewModel

        fun setPlaybackState(state: PlaybackState) {
            viewModel._uiState.value = viewModel._uiState.value.copy(hostPlaybackState = state)
        }
    }

    private fun trackBoundary(
        firstSampleIndex: Long,
        entryId: String = "entry-2",
    ) = FfiPlaylistFrame.TrackBoundary(
        streamId = STREAM_ID,
        firstSampleIndex = firstSampleIndex.toULong(),
        entryId = entryId,
        displayName = if (entryId == "entry-2") "Second track" else "Third track",
    )

    private fun audioFrame(sequence: Long, firstSampleIndex: Long, presentationMs: Long) = FfiPlaylistFrame.Audio(
        streamId = STREAM_ID,
        sequence = sequence.toULong(),
        sampleRate = 48_000u,
        channels = 2u,
        samplesPerPacket = SAMPLES_PER_PACKET.toUInt(),
        firstSampleIndex = firstSampleIndex.toULong(),
        hostPresentationTimeMs = presentationMs.toULong(),
        payload = ByteArray(SAMPLES_PER_PACKET * 4),
    )

    private companion object {
        const val SESSION_ID = "failure-session"
        const val STREAM_ID = "failure-stream"
        const val SAMPLES_PER_PACKET = 960
        const val HOST_START_MS = 400uL
        const val DECODE_FAILURE = "corrupt frame in queued track"
    }
}

private data class AudioCall(val sequence: ULong, val presentationMs: ULong, val sentAtMs: Long)

private data class StopCall(val streamId: String, val hostStopTimeMs: ULong, val sentAtMs: Long)

/** Records host transport sends with the virtual time they happened; one listener always receives them. */
private class RecordingHostTransportHandle(private val clock: () -> Long) : FfiHostTransportHandle(NoHandle) {
    val audio: MutableList<AudioCall> = Collections.synchronizedList(mutableListOf())
    val stops: MutableList<StopCall> = Collections.synchronizedList(mutableListOf())
    val streamStartTimesMs: MutableList<ULong> = Collections.synchronizedList(mutableListOf())
    val boundarySamples: MutableList<ULong> = Collections.synchronizedList(mutableListOf())
    var boundaryDeliveryOverride: FfiHostTransportDelivery? = null
    var boundaryException: Throwable? = null

    override fun pollEvent(timeoutMs: ULong): FfiHostTransportEvent? {
        Thread.sleep(10)
        return null
    }

    override fun broadcastAudio(
        streamId: String,
        sequence: ULong,
        sampleRate: UInt,
        channels: UShort,
        samplesPerPacket: UInt,
        firstSampleIndex: ULong,
        hostPresentationTimeMs: ULong,
        payload: ByteArray,
    ): FfiHostTransportDelivery {
        audio += AudioCall(sequence, hostPresentationTimeMs, clock())
        return DELIVERED
    }

    override fun broadcastStreamStart(
        streamId: String,
        hostStartTimeMs: ULong,
        sampleRate: UInt,
        channels: UShort,
        samplesPerPacket: UInt,
        currentEntryId: String?,
        currentDisplayName: String?,
    ): FfiHostTransportDelivery {
        streamStartTimesMs += hostStartTimeMs
        return DELIVERED
    }

    override fun broadcastTrackBoundary(
        streamId: String,
        firstSampleIndex: ULong,
        entryId: String,
        displayName: String,
    ): FfiHostTransportDelivery {
        boundarySamples += firstSampleIndex
        boundaryException?.let { throw it }
        return boundaryDeliveryOverride ?: DELIVERED
    }

    override fun broadcastStop(streamId: String, hostStopTimeMs: ULong): FfiHostTransportDelivery {
        stops += StopCall(streamId, hostStopTimeMs, clock())
        return DELIVERED
    }

    override fun shutdown() = Unit

    private companion object {
        val DELIVERED = FfiHostTransportDelivery(
            intendedPeers = 1u,
            successfulPeers = 1u,
            failedPeers = 0u,
            bytesSent = 0uL,
        )
    }
}

/** Yields the given frames once each, then fails the way a late Rust decode error surfaces. */
private class FailingPlaylistStreamer(frames: List<FfiPlaylistFrame>) : FfiHostPlaylistStreamer(NoHandle) {
    private val remaining = ArrayDeque(frames)
    var cancelled = false
        private set

    override fun streamStart() = FfiPlaylistStreamStart(
        streamId = "failure-stream",
        hostStartTimeMs = 400uL,
        sampleRate = 48_000u,
        channels = 2u,
        samplesPerPacket = 960u,
        currentEntryId = "entry-1",
        currentDisplayName = "First track",
    )

    override fun nextFrame(timeoutMs: ULong): FfiPlaylistFrame? =
        remaining.removeFirstOrNull() ?: throw FfiHostPlaylistException.Decode("corrupt frame in queued track")

    override fun cancel() {
        cancelled = true
    }
}

/** Accepts monitor packets without opening native Oboe output. */
private class RecordingMonitorPlayback : FfiListenerPlaybackHandle(NoHandle) {
    val reanchoredAtMs = mutableListOf<ULong>()
    var stopped = false
        private set

    override fun submitPacket(packet: FfiAudioPacket, sessionId: String, streamId: String) = Unit

    override fun reanchorPresentationTime(hostStartTimeMs: ULong) {
        reanchoredAtMs += hostStartTimeMs
    }

    override fun stop() {
        stopped = true
    }
}
