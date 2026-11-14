package com.ekkus.silentdisco.app.controller

import com.ekkus.silentdisco.core.audio.AudioFormatSpec
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.rust.HostPlaylistStreamController
import com.ekkus.silentdisco.core.uniffi.FfiCoreSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistSource
import com.ekkus.silentdisco.core.audio.AudioFileAccessException
import com.ekkus.silentdisco.app.PendingHostTrackBoundary
import java.util.concurrent.ConcurrentHashMap
import kotlin.coroutines.CoroutineContext
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

/**
 * The host's playback (God-file refactor spec §4): the staged sources, the
 * running Rust playlist streamer with its stream start and format, the
 * stream-loop and playback-command jobs, and the track-boundary timers.
 *
 * The stream loop body and the live-queue intents still run as
 * `MainViewModel` extensions over this state; they move here next.
 */
internal class HostPlaybackController(
    parentScope: CoroutineScope,
    val staging: HostSourceStaging,
) : ViewModelController(parentScope) {
    /** The Rust decoder/packetizer for the running stream. */
    @Volatile var streamer: HostPlaylistStreamController? = null

    /** The running stream's start announcement, updated as the queue advances and across pauses. */
    @Volatile var streamStart: com.ekkus.silentdisco.core.uniffi.FfiPlaylistStreamStart? = null

    @Volatile var format: AudioFormatSpec? = null

    /** The packets sent last, shown in diagnostics. */
    @Volatile var latestPackets: List<AudioPacket> = emptyList()

    /**
     * Boundaries sent to listeners but not yet reached, by first sample index.
     * A resume re-sends them, re-anchored. Host timing state (spec A.4 T1).
     */
    val pendingBoundaries = ConcurrentHashMap<Long, PendingHostTrackBoundary>()

    /** Timers that advance the host queue at each boundary, by first sample index. */
    val boundaryJobs = ConcurrentHashMap<Long, Job>()

    /** The stream loop sending packets to the monitor and listeners. */
    private var streamJob: Job? = null

    /** The one play, pause, resume or stop command in flight. */
    private var commandJob: Job? = null

    /** Runs [block] as the stream loop, replacing any running one. */
    fun launchStream(context: CoroutineContext, block: suspend CoroutineScope.() -> Unit) {
        streamJob?.cancel()
        streamJob = scope.launch(context, block = block)
    }

    fun cancelStream() {
        streamJob?.cancel()
    }

    /**
     * Runs [block] as the playback command. Without [cancelExisting], a
     * command still in flight wins and this returns false without starting.
     */
    fun launchCommand(cancelExisting: Boolean, block: suspend CoroutineScope.() -> Unit): Boolean {
        val existing = commandJob
        if (cancelExisting) {
            existing?.cancel()
        } else if (existing?.isActive == true) {
            return false
        }
        val job = scope.launch(block = block)
        commandJob = job
        job.invokeOnCompletion { if (commandJob === job) commandJob = null }
        return true
    }

    fun cancelCommand() {
        commandJob?.cancel()
    }

    /** Runs [block] as the timer for the boundary at [sampleIndex], replacing any timer already there. */
    fun launchBoundaryTimer(sampleIndex: Long, context: CoroutineContext, block: suspend CoroutineScope.() -> Unit) {
        val job = scope.launch(context, block = block)
        boundaryJobs.put(sampleIndex, job)?.cancel()
        job.invokeOnCompletion { boundaryJobs.remove(sampleIndex, job) }
    }

    fun cancelBoundaryTimers() {
        val pendingJobs = boundaryJobs.values.toList()
        boundaryJobs.clear()
        pendingJobs.forEach(Job::cancel)
    }

    /** Cancels the timers of boundaries at or after [firstUnsentSampleIndex]: no listener will hear them. */
    fun cancelBoundaryTimersFrom(firstUnsentSampleIndex: Long) {
        boundaryJobs.keys
            .filter { it >= firstUnsentSampleIndex }
            .forEach { sampleIndex -> boundaryJobs.remove(sampleIndex)?.cancel() }
    }

    /** Retargets the running streamer's upcoming tracks from an authoritative actor snapshot. */
    fun retarget(snapshot: FfiCoreSnapshot) {
        val running = streamer ?: return
        val upcoming = snapshot.hostQueue.drop(1).map { entry ->
            val staged = staging.stagedFile(entry.source.sourceId)
                ?: throw AudioFileAccessException("${entry.source.displayName}: staged audio is unavailable")
            FfiPlaylistSource(entry.entryId, entry.source.displayName, staged.absolutePath)
        }
        running.replaceUpcoming(upcoming)
    }

    /** Cancels and closes the streamer and forgets the stream; staged sources stay. */
    fun clearStream() {
        pendingBoundaries.clear()
        val running = streamer
        streamer = null
        runCatching { running?.cancel() }
        runCatching { running?.close() }
        format = null
        streamStart = null
        latestPackets = emptyList()
    }

    /** `MainViewModel.onCleared` releases the staged sources at its original point; this covers the rest. */
    override fun release() {
        cancelCommand()
        cancelStream()
        cancelBoundaryTimers()
        clearStream()
        staging.releaseAll()
    }
}
