package com.ekkus.silentdisco.app.controller

import com.ekkus.silentdisco.core.audio.AudioFormatSpec
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistFrame
import com.ekkus.silentdisco.core.rust.HostPlaylistStreamController
import com.ekkus.silentdisco.core.uniffi.FfiCoreSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistSource
import com.ekkus.silentdisco.core.audio.AudioFileAccessException
import com.ekkus.silentdisco.core.uniffi.FfiHostBoundarySchedule
import kotlin.coroutines.CoroutineContext
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

/** A stream opened and ready to play. */
internal data class PreparedHostStream(
    val streamer: HostPlaylistStreamController,
    val format: AudioFormatSpec,
    val firstPacket: AudioPacket,
)

/**
 * The host's playback (God-file refactor spec §4): the staged sources,
 * stream preparation, the running Rust playlist streamer with its stream
 * start and format, the stream-loop and playback-command jobs, and the
 * boundary poller over the Rust boundary schedule. The loop itself is [HostStreamLoop].
 *
 * Play, pause, resume and stop stay with the coordinator: each waits for the
 * host core's authority, then drives this controller, the monitor, the
 * transport and the listener role together.
 */
internal class HostPlaybackController(
    parentScope: CoroutineScope,
    val staging: HostSourceStaging,
    /**
     * Opens the Rust host boundary schedule for a stream (its original host
     * start time, its sample rate). The desktop host uses the same rule, so
     * Kotlin never computes a boundary deadline. JVM tests inject a fake.
     */
    private val newBoundarySchedule: (ULong, UInt) -> FfiHostBoundarySchedule = ::FfiHostBoundarySchedule,
) : ViewModelController(parentScope) {
    /** The host's stream id, from session start until the session ends. */
    @Volatile var streamId: StreamId? = null

    /** The Rust decoder/packetizer for the running stream. */
    @Volatile var streamer: HostPlaylistStreamController? = null

    /** The running stream's start announcement, updated as the queue advances and across pauses. */
    @Volatile var streamStart: com.ekkus.silentdisco.core.uniffi.FfiPlaylistStreamStart? = null

    @Volatile var format: AudioFormatSpec? = null

    /** The packets sent last, shown in diagnostics. */
    @Volatile var latestPackets: List<AudioPacket> = emptyList()

    /**
     * The running stream's boundaries sent to listeners but not yet applied
     * to the host queue, owned by Rust (gapless TODO §5). A resume re-sends
     * them. Replaced when a stream is prepared; null without a stream.
     */
    @Volatile var boundarySchedule: FfiHostBoundarySchedule? = null
        private set

    /**
     * The pause offset the stream loop has applied to the running stream's
     * audio, in ms. The boundary poller passes exactly this to the Rust
     * schedule, so a boundary moves by what the audio moved (gapless TODO §4:
     * one pause measurement, as on the desktop). Written only by the stream loop.
     */
    @Volatile var presentationOffsetMs: Long = 0L

    /**
     * True from when the stream loop sees a pause until it has added that
     * pause to [presentationOffsetMs]; the boundary poller waits meanwhile.
     */
    @Volatile var pauseInProgress: Boolean = false

    /** Applies due boundaries to the host queue while the stream runs. */
    @Volatile var boundaryPoller: Job? = null
        private set

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

    /** Replaces the boundary schedule with a new, empty one for a stream starting at [hostStartTimeMs]. */
    fun openBoundarySchedule(hostStartTimeMs: ULong, sampleRate: UInt): FfiHostBoundarySchedule =
        newBoundarySchedule(hostStartTimeMs, sampleRate).also { boundarySchedule = it }

    /** Runs [block] as the boundary poller unless one is already running. */
    fun ensureBoundaryPoller(context: CoroutineContext, block: suspend CoroutineScope.() -> Unit) {
        if (boundaryPoller?.isActive == true) return
        boundaryPoller = scope.launch(context, block = block)
    }

    /** Stops applying boundaries: the stream stopped, failed or ended. */
    fun cancelBoundaryPoller() {
        boundaryPoller?.cancel()
        boundaryPoller = null
    }

    /**
     * Withdraws the boundaries at or after [firstUnsentSampleIndex]: no
     * listener will hear them, so the host queue must not advance to them.
     */
    fun withdrawBoundariesFrom(firstUnsentSampleIndex: Long) {
        boundarySchedule?.withdrawFrom(firstUnsentSampleIndex.toULong())
    }

    /**
     * Prepares [streamId] to play: forgets the last stream's boundaries,
     * opens the streamer with [open] and reads its first packet. A failure
     * to read that packet clears the stream before it propagates.
     */
    suspend fun prepareStream(
        sessionId: SessionId,
        streamId: StreamId,
        open: suspend () -> HostPlaylistStreamController,
    ): PreparedHostStream {
        forgetBoundaries()
        this.streamId = streamId
        val opened = open()
        streamer = opened
        val start = opened.streamStart
        streamStart = start
        try {
            openBoundarySchedule(start.hostStartTimeMs, start.sampleRate)
        } catch (error: Throwable) {
            clearStream()
            throw error
        }
        val streamFormat = AudioFormatSpec(start.sampleRate.toInt(), start.channels.toInt())
        format = streamFormat
        val firstPacket = try {
            opened.nextAudioPacket(sessionId, streamId)
        } catch (error: Throwable) {
            clearStream()
            throw error
        }
        latestPackets = listOf(firstPacket)
        return PreparedHostStream(opened, streamFormat, firstPacket)
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
        forgetBoundaries()
        val running = streamer
        streamer = null
        runCatching { running?.cancel() }
        runCatching { running?.close() }
        format = null
        streamStart = null
        latestPackets = emptyList()
    }

    /**
     * Drops the stream's schedule. It is not closed here: a cancelled poller
     * may still be inside a call on it, and UniFFI's cleaner frees it once
     * nothing refers to it. The poller re-reads [boundarySchedule] each turn.
     */
    private fun forgetBoundaries() {
        boundarySchedule = null
    }

    /** `MainViewModel.onCleared` releases the staged sources at its original point; this covers the rest. */
    override fun release() {
        cancelCommand()
        cancelStream()
        cancelBoundaryPoller()
        clearStream()
        staging.releaseAll()
    }
}

private suspend fun HostPlaylistStreamController.nextAudioPacket(sessionId: SessionId, streamId: StreamId): AudioPacket {
    while (true) {
        when (val frame = nextFrame(2_000uL)) {
            null -> continue
            FfiPlaylistFrame.End -> error("Playlist ended before producing audio")
            is FfiPlaylistFrame.TrackBoundary -> error("Playlist began with an unexpected track boundary")
            is FfiPlaylistFrame.Audio -> return AudioPacket(
                version = 3,
                sessionId = sessionId,
                streamId = StreamId(frame.streamId),
                sequenceNumber = frame.sequence.toLong(),
                codec = "pcm16le",
                sampleRate = frame.sampleRate.toInt(),
                channelCount = frame.channels.toInt(),
                samplesPerPacket = frame.samplesPerPacket.toInt(),
                firstSampleIndex = frame.firstSampleIndex.toLong(),
                hostPresentationTimeMs = frame.hostPresentationTimeMs.toLong(),
                payload = frame.payload,
            ).also {
                require(it.streamId == streamId) { "playlist frame belongs to another stream" }
            }
        }
    }
}
