package com.ekkus.silentdisco.app.controller

import com.ekkus.silentdisco.app.PendingHostTrackBoundary
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
}
