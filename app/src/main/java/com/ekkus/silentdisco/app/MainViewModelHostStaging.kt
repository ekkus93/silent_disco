package com.ekkus.silentdisco.app

import android.app.Application
import android.net.Uri
import android.os.StatFs
import com.ekkus.silentdisco.core.audio.AudioFileAccessException
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.ekkus.silentdisco.core.rust.HostPlaylistStreamController
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistFrame
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistSource
import com.ekkus.silentdisco.core.uniffi.FfiCoreSnapshot
import java.io.File
import java.io.InputStream
import java.io.IOException
import java.util.UUID
import kotlinx.coroutines.ensureActive
import kotlin.coroutines.coroutineContext

private const val ANDROID_MAX_SOURCE_BYTES = 8L * 1024 * 1024 * 1024
private const val ANDROID_MAX_QUEUE_SOURCE_BYTES = 16L * 1024 * 1024 * 1024

/** Copies and fully decodes one provider-owned source before queue acceptance. */
internal suspend fun MainViewModel.stageAndValidateHostSource(audio: SelectedAudioFile): File {
    val sourceId = stableOpaqueAudioSourceId(audio.uri.toString())
    stagedAudioFilesBySourceId[sourceId]?.takeIf(File::isFile)?.let { return it }
    val extension = audio.displayName.substringAfterLast('.', "audio")
        .filter(Char::isLetterOrDigit).take(8).ifEmpty { "audio" }
    val staged = try {
        File.createTempFile(
            "silent-disco-source-${UUID.randomUUID()}-",
            ".$extension",
            getApplication<Application>().cacheDir,
        )
    } catch (error: IOException) {
        throw AudioFileAccessException("${audio.displayName}: not enough device storage to stage this song", error)
    }
    try {
        copyUriToBoundedFile(audio.uri, staged, audio.displayName, audio.sizeBytes)
        val totalBytes = stagedAudioFilesBySourceId.values.filter(File::isFile).sumOf(File::length) + staged.length()
        if (totalBytes > ANDROID_MAX_QUEUE_SOURCE_BYTES) {
            throw AudioFileAccessException("Queue sources exceed the 16 GiB total limit")
        }
        validateStagedAudio(audio.displayName, staged)
        stagedAudioFilesBySourceId[sourceId] = staged
        return staged
    } catch (error: Throwable) {
        staged.delete()
        if (error.message?.contains(audio.displayName) == true) throw error
        throw AudioFileAccessException("${audio.displayName}: ${error.message ?: "audio validation failed"}", error)
    }
}

private suspend fun MainViewModel.validateStagedAudio(displayName: String, staged: File) {
    val stream = HostPlaylistStreamController.open(
        sessionId = "validation-${UUID.randomUUID()}",
        streamId = "validation-${UUID.randomUUID()}",
        hostStartTimeMs = 0uL,
        sources = listOf(
            FfiPlaylistSource(
                entryId = "validation-entry",
                displayName = displayName,
                stagedPath = staged.absolutePath,
            ),
        ),
    )
    try {
        while (true) {
            coroutineContext.ensureActive()
            when (stream.nextFrame(250uL)) {
                null -> continue
                FfiPlaylistFrame.End -> return
                is FfiPlaylistFrame.Audio -> Unit
                is FfiPlaylistFrame.TrackBoundary -> {
                    throw AudioFileAccessException("$displayName: unexpected boundary while validating source")
                }
            }
        }
    } finally {
        runCatching { stream.cancel() }
        stream.close()
    }
}

/** Resolves opaque queue source IDs to staged files already validated by Rust. */
internal suspend fun MainViewModel.stageHostPlaylistSources(): List<FfiPlaylistSource> {
    val selected = listOfNotNull(_uiState.value.hostForm.selectedAudio) +
        _uiState.value.hostForm.additionalAudio
    val uriBySourceId = mutableMapOf<String, Uri>()
    val selectedBySourceId = mutableMapOf<String, SelectedAudioFile>()
    selected.forEach { audio ->
        val id = stableOpaqueAudioSourceId(audio.uri.toString())
        uriBySourceId[id] = audio.uri
        selectedBySourceId[id] = audio
    }
    uriBySourceId.putAll(liveQueueSourceUris)

    val queue = _uiState.value.hostQueue
    val entries = if (queue.isNotEmpty()) {
        queue
    } else {
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

    return entries.map { entry ->
        coroutineContext.ensureActive()
        val staged = stagedAudioFilesBySourceId[entry.sourceId]?.takeIf(File::isFile)
            ?: run {
                val audio = selectedBySourceId[entry.sourceId] ?: SelectedAudioFile(
                    uri = uriBySourceId[entry.sourceId]
                        ?: throw AudioFileAccessException("${entry.displayName}: source permission is unavailable"),
                    displayName = entry.displayName,
                    mimeType = null,
                    sizeBytes = entry.byteLength,
                )
                stageAndValidateHostSource(audio)
            }
        FfiPlaylistSource(entry.entryId, entry.displayName, staged.absolutePath)
    }
}

/** Retargets the running Rust packetizer from an authoritative actor snapshot. */
internal fun MainViewModel.retargetActiveHostPlaylist(snapshot: FfiCoreSnapshot) {
    val streamer = hostPlaylistStreamer ?: return
    val upcoming = snapshot.hostQueue.drop(1).map { entry ->
        val staged = stagedAudioFilesBySourceId[entry.source.sourceId]
            ?.takeIf(File::isFile)
            ?: throw AudioFileAccessException("${entry.source.displayName}: staged audio is unavailable")
        FfiPlaylistSource(entry.entryId, entry.source.displayName, staged.absolutePath)
    }
    streamer.replaceUpcoming(upcoming)
}

private suspend fun MainViewModel.copyUriToBoundedFile(
    uri: Uri,
    destination: File,
    displayName: String,
    expectedSize: Long?,
) {
    if (expectedSize != null && expectedSize > ANDROID_MAX_SOURCE_BYTES) {
        throw AudioFileAccessException("$displayName: file exceeds the 8 GiB source limit")
    }
    val resolver = getApplication<Application>().contentResolver
    val availableBytes = StatFs(destination.parentFile!!.absolutePath).availableBytes
    if (expectedSize != null && expectedSize > availableBytes) {
        throw AudioFileAccessException("$displayName: not enough device storage to stage this song")
    }
    copyProviderAudioToBoundedFile(
        openInputStream = { resolver.openInputStream(uri) },
        destination = destination,
        displayName = displayName,
        expectedSize = expectedSize,
        availableBytes = { StatFs(destination.parentFile!!.absolutePath).availableBytes },
    )
}

internal suspend fun copyProviderAudioToBoundedFile(
    openInputStream: () -> InputStream?,
    destination: File,
    displayName: String,
    expectedSize: Long?,
    availableBytes: () -> Long,
) {
    val input = try {
        openInputStream()
    } catch (error: SecurityException) {
        throw AudioFileAccessException("$displayName: access to this song was revoked", error)
    } ?: throw AudioFileAccessException("$displayName: could not open the selected file")
    try {
        copyAudioStreamToBoundedFile(
            input = input,
            destination = destination,
            displayName = displayName,
            expectedSize = expectedSize,
            availableBytes = availableBytes,
        )
    } catch (error: SecurityException) {
        throw AudioFileAccessException("$displayName: access to this song was revoked", error)
    }
}

internal suspend fun copyAudioStreamToBoundedFile(
    input: InputStream,
    destination: File,
    displayName: String,
    expectedSize: Long?,
    availableBytes: () -> Long,
) {
    if (expectedSize != null && expectedSize > ANDROID_MAX_SOURCE_BYTES) {
        throw AudioFileAccessException("$displayName: file exceeds the 8 GiB source limit")
    }
    val availableAtStart = availableBytes()
    if (expectedSize != null && expectedSize > availableAtStart) {
        throw AudioFileAccessException("$displayName: not enough device storage to stage this song")
    }
    try {
        input.use { source ->
            destination.outputStream().buffered().use { target ->
                val buffer = ByteArray(64 * 1024)
                var copied = 0L
                while (true) {
                    coroutineContext.ensureActive()
                    val count = source.read(buffer)
                    if (count < 0) break
                    copied = Math.addExact(copied, count.toLong())
                    if (copied > ANDROID_MAX_SOURCE_BYTES) {
                        throw AudioFileAccessException("$displayName: file exceeds the 8 GiB source limit")
                    }
                    if (count.toLong() > availableBytes()) {
                        throw AudioFileAccessException("$displayName: device storage filled while staging")
                    }
                    target.write(buffer, 0, count)
                }
                if (copied == 0L) throw AudioFileAccessException("$displayName: file is empty")
                if (expectedSize != null && copied != expectedSize) {
                    throw AudioFileAccessException("$displayName: source length changed while staging")
                }
            }
        }
    } catch (error: IOException) {
        throw AudioFileAccessException("$displayName: could not stage selected audio", error)
    }
}

internal fun MainViewModel.clearHostPlaylistStream() {
    hostPendingTrackBoundaries.clear()
    val streamer = hostPlaylistStreamer
    hostPlaylistStreamer = null
    runCatching { streamer?.cancel() }
    runCatching { streamer?.close() }
    hostPlaylistFormat = null
    hostPlaylistStreamStart = null
    latestPackets = emptyList()
}

internal fun MainViewModel.releaseStagedHostSources() {
    clearHostPlaylistStream()
    stagedAudioFilesBySourceId.values.forEach(File::delete)
    stagedAudioFilesBySourceId.clear()
}
