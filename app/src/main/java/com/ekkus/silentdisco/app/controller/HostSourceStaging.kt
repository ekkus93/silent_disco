package com.ekkus.silentdisco.app.controller

import android.net.Uri
import android.os.StatFs
import com.ekkus.silentdisco.app.HostQueueItem
import com.ekkus.silentdisco.app.stableOpaqueAudioSourceId
import com.ekkus.silentdisco.core.audio.AudioFileAccessException
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.ekkus.silentdisco.core.rust.HostPlaylistStreamController
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistFrame
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistSource
import java.io.File
import java.io.IOException
import java.io.InputStream
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import kotlin.coroutines.coroutineContext
import kotlinx.coroutines.ensureActive

private const val ANDROID_MAX_SOURCE_BYTES = 8L * 1024 * 1024 * 1024
private const val ANDROID_MAX_QUEUE_SOURCE_BYTES = 16L * 1024 * 1024 * 1024
private const val STAGED_SOURCE_PREFIX = "silent-disco-source-"

/**
 * The host's staged audio (God-file refactor spec §4, HostPlayback): each
 * provider-owned song is copied into app storage under bounded size and
 * storage checks, then fully decoded once by Rust before the queue accepts
 * it. The streamer only ever reads these staged copies.
 *
 * Also remembers the URIs of songs added to the live queue, so a later
 * restart can stage them again. The provider and storage seams are
 * injectable for JVM tests.
 */
internal class HostSourceStaging(
    private val cacheDir: () -> File,
    private val openInput: (Uri) -> InputStream?,
    private val availableBytes: (File) -> Long = { StatFs(it.absolutePath).availableBytes },
    private val validate: suspend (displayName: String, staged: File) -> Unit = ::validateWithRustDecoder,
    private val sourceIdOf: (Uri) -> String = { stableOpaqueAudioSourceId(it.toString()) },
) {
    private val stagedBySourceId = ConcurrentHashMap<String, File>()

    /** Live-queue song URIs, by source id. */
    val liveQueueUris = ConcurrentHashMap<String, Uri>()

    fun isStaged(sourceId: String): Boolean = stagedBySourceId.containsKey(sourceId)

    /** The staged file for [sourceId], if it is still on disk. */
    fun stagedFile(sourceId: String): File? = stagedBySourceId[sourceId]?.takeIf(File::isFile)

    /** For tests and teardown checks. */
    val stagedSourceIds: Set<String> get() = stagedBySourceId.keys

    /** Copies and fully decodes one provider-owned source before queue acceptance. */
    suspend fun stage(audio: SelectedAudioFile): File {
        val sourceId = sourceIdOf(audio.uri)
        stagedFile(sourceId)?.let { return it }
        val extension = audio.displayName.substringAfterLast('.', "audio")
            .filter(Char::isLetterOrDigit).take(8).ifEmpty { "audio" }
        val staged = try {
            File.createTempFile("$STAGED_SOURCE_PREFIX${UUID.randomUUID()}-", ".$extension", cacheDir())
        } catch (error: IOException) {
            throw AudioFileAccessException("${audio.displayName}: not enough device storage to stage this song", error)
        }
        try {
            copy(audio.uri, staged, audio.displayName, audio.sizeBytes)
            val totalBytes = stagedBySourceId.values.filter(File::isFile).sumOf(File::length) + staged.length()
            if (totalBytes > ANDROID_MAX_QUEUE_SOURCE_BYTES) {
                throw AudioFileAccessException("Queue sources exceed the 16 GiB total limit")
            }
            validate(audio.displayName, staged)
            stagedBySourceId[sourceId] = staged
            return staged
        } catch (error: Throwable) {
            staged.delete()
            if (error.message?.contains(audio.displayName) == true) throw error
            throw AudioFileAccessException("${audio.displayName}: ${error.message ?: "audio validation failed"}", error)
        }
    }

    /**
     * Resolves queue entries to staged files, staging any not yet staged.
     * [selected] are the setup songs, which carry their own metadata; any
     * other source must be a live-queue song whose URI is remembered.
     */
    suspend fun resolve(entries: List<HostQueueItem>, selected: List<SelectedAudioFile>): List<FfiPlaylistSource> {
        val uriBySourceId = mutableMapOf<String, Uri>()
        val selectedBySourceId = mutableMapOf<String, SelectedAudioFile>()
        selected.forEach { audio ->
            val id = sourceIdOf(audio.uri)
            uriBySourceId[id] = audio.uri
            selectedBySourceId[id] = audio
        }
        uriBySourceId.putAll(liveQueueUris)
        return entries.map { entry ->
            coroutineContext.ensureActive()
            val staged = stagedFile(entry.sourceId) ?: run {
                val audio = selectedBySourceId[entry.sourceId] ?: SelectedAudioFile(
                    uri = uriBySourceId[entry.sourceId]
                        ?: throw AudioFileAccessException("${entry.displayName}: source permission is unavailable"),
                    displayName = entry.displayName,
                    mimeType = null,
                    sizeBytes = entry.byteLength,
                )
                stage(audio)
            }
            FfiPlaylistSource(entry.entryId, entry.displayName, staged.absolutePath)
        }
    }

    /** Deletes the staged copies of [sourceIds], e.g. ones staged by a step that then failed. */
    fun discard(sourceIds: Collection<String>) {
        sourceIds.mapNotNull(stagedBySourceId::remove).forEach { it.delete() }
    }

    /** Deletes every staged copy. */
    fun releaseAll() {
        stagedBySourceId.values.forEach(File::delete)
        stagedBySourceId.clear()
    }

    /** Deletes staged copies a previous process left behind. */
    fun deleteOrphans() {
        val dir = runCatching(cacheDir).getOrNull() ?: return
        dir.listFiles()?.filter { it.name.startsWith(STAGED_SOURCE_PREFIX) }?.forEach { it.delete() }
    }

    /** Stages one file directly; for tests that need a staged source without a provider. */
    internal fun putStaged(sourceId: String, file: File) {
        stagedBySourceId[sourceId] = file
    }

    private suspend fun copy(uri: Uri, destination: File, displayName: String, expectedSize: Long?) {
        if (expectedSize != null && expectedSize > ANDROID_MAX_SOURCE_BYTES) {
            throw AudioFileAccessException("$displayName: file exceeds the 8 GiB source limit")
        }
        val directory = destination.parentFile!!
        if (expectedSize != null && expectedSize > availableBytes(directory)) {
            throw AudioFileAccessException("$displayName: not enough device storage to stage this song")
        }
        copyProviderAudioToBoundedFile(
            openInputStream = { openInput(uri) },
            destination = destination,
            displayName = displayName,
            expectedSize = expectedSize,
            availableBytes = { availableBytes(directory) },
        )
    }
}

/** Decodes [staged] end to end with the Rust streamer; any decode failure rejects the song. */
private suspend fun validateWithRustDecoder(displayName: String, staged: File) {
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
