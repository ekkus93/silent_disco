package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.audio.AudioFileAccessException
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.ekkus.silentdisco.core.uniffi.FfiCoreSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistSource
import java.io.File
import java.util.UUID

/** Copies and fully decodes one provider-owned source before queue acceptance. */
internal suspend fun MainViewModel.stageAndValidateHostSource(audio: SelectedAudioFile): File =
    hostPlayback.staging.stage(audio)

/** Resolves the host queue (or the single chosen song) to staged files already validated by Rust. */
internal suspend fun MainViewModel.stageHostPlaylistSources(): List<FfiPlaylistSource> {
    val selected = listOfNotNull(_uiState.value.hostForm.selectedAudio) + _uiState.value.hostForm.additionalAudio
    val queue = _uiState.value.hostQueue
    val entries = queue.ifEmpty {
        val fallback = _uiState.value.hostForm.selectedAudio
            ?: throw AudioFileAccessException("Choose a song before starting playback")
        listOf(
            HostQueueItem(
                entryId = "android-single-${UUID.randomUUID()}",
                sourceId = stableOpaqueAudioSourceId(fallback.uri.toString()),
                displayName = fallback.displayName,
                byteLength = fallback.sizeBytes,
                durationMs = null,
            ),
        )
    }
    return hostPlayback.staging.resolve(entries, selected)
}

/** Retargets the running Rust packetizer from an authoritative actor snapshot. */
internal fun MainViewModel.retargetActiveHostPlaylist(snapshot: FfiCoreSnapshot) = hostPlayback.retarget(snapshot)

internal fun MainViewModel.clearHostPlaylistStream() {
    hostPendingTrackBoundaries.clear()
    hostPlayback.clearStream()
}

internal fun MainViewModel.releaseStagedHostSources() {
    clearHostPlaylistStream()
    hostPlayback.staging.releaseAll()
}
