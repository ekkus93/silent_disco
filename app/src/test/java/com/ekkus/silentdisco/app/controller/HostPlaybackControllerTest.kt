package com.ekkus.silentdisco.app.controller

import com.ekkus.silentdisco.app.PendingHostTrackBoundary
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.rust.HostPlaylistStreamController
import com.ekkus.silentdisco.core.uniffi.FfiHostPlaylistStreamer
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistFrame
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistStreamStart
import com.ekkus.silentdisco.core.uniffi.NoHandle
import com.google.common.truth.Truth.assertThat
import java.io.File
import java.nio.file.Files
import kotlin.coroutines.EmptyCoroutineContext
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.cancel
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.After
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class HostPlaybackControllerTest {
    private val dir: File = Files.createTempDirectory("host-playback-test").toFile()

    @After
    fun deleteDir() {
        dir.deleteRecursively()
    }

    private fun TestScope.controller(): Pair<HostPlaybackController, CoroutineScope> {
        val parent = CoroutineScope(SupervisorJob() + StandardTestDispatcher(testScheduler))
        val staging = HostSourceStaging(cacheDir = { dir }, openInput = { null }, validate = { _, _ -> })
        return HostPlaybackController(parent, staging) to parent
    }

    @Test
    fun aSecondCommandWaitsForTheFirstUnlessItReplacesIt() = runTest {
        val (playback, parent) = controller()
        var first: Job? = null
        assertThat(playback.launchCommand(cancelExisting = false) { first = coroutineContext[Job]; awaitCancellation() })
            .isTrue()
        runCurrent()

        var ranSecond = false
        assertThat(playback.launchCommand(cancelExisting = false) { ranSecond = true }).isFalse()
        runCurrent()
        assertThat(ranSecond).isFalse()

        assertThat(playback.launchCommand(cancelExisting = true) { ranSecond = true }).isTrue()
        runCurrent()
        assertThat(first!!.isCancelled).isTrue()
        assertThat(ranSecond).isTrue()

        // The finished command no longer blocks the next one.
        assertThat(playback.launchCommand(cancelExisting = false) {}).isTrue()
        parent.cancel()
    }

    @Test
    fun aBoundaryTimerReplacesTheOneAtItsSampleAndForgetsItselfWhenDone() = runTest {
        val (playback, parent) = controller()
        var replaced: Job? = null
        playback.launchBoundaryTimer(960L, EmptyCoroutineContext) { replaced = coroutineContext[Job]; awaitCancellation() }
        runCurrent()

        playback.launchBoundaryTimer(960L, EmptyCoroutineContext) {}
        runCurrent()

        assertThat(replaced!!.isCancelled).isTrue()
        assertThat(playback.boundaryJobs).isEmpty()
        parent.cancel()
    }

    @Test
    fun timersOfBoundariesNoListenerWillHearAreCancelledAndEarlierOnesKept() = runTest {
        val (playback, parent) = controller()
        listOf(480L, 960L, 1_440L).forEach { index ->
            playback.launchBoundaryTimer(index, EmptyCoroutineContext) { awaitCancellation() }
        }
        runCurrent()

        playback.cancelBoundaryTimersFrom(960L)
        assertThat(playback.boundaryJobs.keys).containsExactly(480L)

        playback.cancelBoundaryTimers()
        assertThat(playback.boundaryJobs).isEmpty()
        parent.cancel()
    }

    @Test
    fun clearingTheStreamForgetsPendingBoundariesAndReleaseCancelsTheStream() = runTest {
        val (playback, parent) = controller()
        playback.pendingBoundaries[960L] = PendingHostTrackBoundary("stream-1", 960L, "entry-2", "Second")
        var stream: Job? = null
        playback.launchStream(EmptyCoroutineContext) { stream = coroutineContext[Job]; awaitCancellation() }
        runCurrent()

        playback.clearStream()
        assertThat(playback.pendingBoundaries).isEmpty()
        assertThat(stream!!.isActive).isTrue()

        playback.close()
        runCurrent()
        assertThat(stream!!.isCancelled).isTrue()
        parent.cancel()
    }

    private class ScriptedStreamer(frames: List<FfiPlaylistFrame>) : FfiHostPlaylistStreamer(NoHandle) {
        private val remaining = ArrayDeque(frames)
        var cancelled = false

        override fun streamStart() = FfiPlaylistStreamStart(
            streamId = "stream-1",
            hostStartTimeMs = 500uL,
            sampleRate = 48_000u,
            channels = 2u,
            samplesPerPacket = 960u,
            currentEntryId = "entry-1",
            currentDisplayName = "First",
        )

        override fun nextFrame(timeoutMs: ULong): FfiPlaylistFrame? = remaining.removeFirstOrNull()

        override fun cancel() {
            cancelled = true
        }

        override fun close() = Unit
    }

    private fun audio(stream: String = "stream-1") = FfiPlaylistFrame.Audio(
        streamId = stream,
        sequence = 0uL,
        sampleRate = 48_000u,
        channels = 2u,
        samplesPerPacket = 960u,
        firstSampleIndex = 0uL,
        hostPresentationTimeMs = 500uL,
        payload = ByteArray(960 * 4),
    )

    private val session = SessionId("session-1")
    private val stream = StreamId("stream-1")

    @Test
    fun preparingAStreamOpensItAndKeepsItsFormatAndFirstPacket() = runTest {
        val (playback, parent) = controller()
        playback.pendingBoundaries[960L] = PendingHostTrackBoundary("old", 960L, "entry-0", "Old")
        val streamer = HostPlaylistStreamController(ScriptedStreamer(listOf(audio())))

        val prepared = playback.prepareStream(session, stream) { streamer }

        assertThat(prepared.streamer).isSameInstanceAs(streamer)
        assertThat(prepared.format.sampleRate).isEqualTo(48_000)
        assertThat(prepared.firstPacket.hostPresentationTimeMs).isEqualTo(500L)
        assertThat(playback.streamId).isEqualTo(stream)
        assertThat(playback.streamer).isSameInstanceAs(streamer)
        assertThat(playback.streamStart?.currentEntryId).isEqualTo("entry-1")
        assertThat(playback.latestPackets).containsExactly(prepared.firstPacket)
        assertThat(playback.pendingBoundaries).isEmpty()
        parent.cancel()
    }

    @Test
    fun aStreamThatDoesNotStartWithItsOwnAudioIsClearedAndTheFailurePropagates() = runTest {
        val (playback, parent) = controller()
        listOf(
            listOf<FfiPlaylistFrame>(FfiPlaylistFrame.End) to "ended before producing audio",
            listOf(audio(stream = "other")) to "belongs to another stream",
        ).forEach { (frames, message) ->
            val fake = ScriptedStreamer(frames)
            val failure = runCatching {
                playback.prepareStream(session, stream) { HostPlaylistStreamController(fake) }
            }.exceptionOrNull()

            assertThat(failure).hasMessageThat().contains(message)
            assertThat(fake.cancelled).isTrue()
            assertThat(playback.streamer).isNull()
            assertThat(playback.format).isNull()
            assertThat(playback.streamStart).isNull()
        }
        parent.cancel()
    }
}
