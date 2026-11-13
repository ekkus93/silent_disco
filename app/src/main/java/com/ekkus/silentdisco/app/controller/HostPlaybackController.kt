package com.ekkus.silentdisco.app.controller

import com.ekkus.silentdisco.core.audio.AudioFormatSpec
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.rust.HostPlaylistStreamController
import com.ekkus.silentdisco.core.uniffi.FfiCoreSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistSource
import com.ekkus.silentdisco.core.audio.AudioFileAccessException
import kotlinx.coroutines.CoroutineScope

/**
 * The host's playback (God-file refactor spec §4): the staged sources and
 * the running Rust playlist streamer with its stream start and format.
 *
 * The stream loop, live-queue intents and track boundaries still run as
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
        clearStream()
        staging.releaseAll()
    }
}
