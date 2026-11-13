package com.ekkus.silentdisco.app.controller

import android.net.Uri
import com.ekkus.silentdisco.app.HostQueueItem
import com.ekkus.silentdisco.core.audio.AudioFileAccessException
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.google.common.truth.Truth.assertThat
import java.io.ByteArrayInputStream
import java.io.File
import java.nio.file.Files
import kotlinx.coroutines.test.runTest
import org.junit.After
import org.junit.Assert.assertThrows
import org.junit.Test
import org.mockito.Mockito.mock

class HostSourceStagingTest {
    private val dir: File = Files.createTempDirectory("staging-test").toFile()
    private val contents = mutableMapOf<Uri, ByteArray>()
    private val ids = mutableMapOf<Uri, String>()
    private val validated = mutableListOf<String>()
    private var validationFailure: Exception? = null

    @After
    fun deleteDir() {
        dir.deleteRecursively()
    }

    private fun staging() = HostSourceStaging(
        cacheDir = { dir },
        openInput = { uri -> contents[uri]?.let(::ByteArrayInputStream) },
        availableBytes = { Long.MAX_VALUE },
        validate = { name, _ ->
            validated += name
            validationFailure?.let { throw it }
        },
        sourceIdOf = { ids.getValue(it) },
    )

    private fun song(id: String, bytes: Int = 16): SelectedAudioFile {
        val uri = mock(Uri::class.java)
        ids[uri] = id
        contents[uri] = ByteArray(bytes) { 1 }
        return SelectedAudioFile(uri = uri, displayName = "$id.wav", mimeType = null, sizeBytes = bytes.toLong())
    }

    @Test
    fun stagingCopiesValidatesOnceAndReusesTheCopy() = runTest {
        val staging = staging()
        val audio = song("a")

        val first = staging.stage(audio)
        val second = staging.stage(audio)

        assertThat(second).isEqualTo(first)
        assertThat(first.length()).isEqualTo(16L)
        assertThat(validated).containsExactly("a.wav")
        assertThat(staging.isStaged("a")).isTrue()
    }

    @Test
    fun aSongThatFailsValidationLeavesNothingBehind() = runTest {
        val staging = staging()
        validationFailure = IllegalStateException("not audio")

        val failure = runCatching { staging.stage(song("bad")) }.exceptionOrNull()

        assertThat(failure).isInstanceOf(AudioFileAccessException::class.java)
        assertThat(failure).hasMessageThat().isEqualTo("bad.wav: not audio")
        assertThat(staging.isStaged("bad")).isFalse()
        assertThat(dir.listFiles()).isEmpty()
    }

    @Test
    fun resolveStagesALiveQueueSongFromItsRememberedUriAndRejectsAnUnknownOne() = runTest {
        val staging = staging()
        val live = song("live")
        staging.liveQueueUris["live"] = live.uri
        val entry = HostQueueItem("entry-1", "live", "Live", byteLength = 16L, durationMs = null)

        val sources = staging.resolve(listOf(entry), selected = emptyList())

        assertThat(sources.single().entryId).isEqualTo("entry-1")
        assertThat(File(sources.single().stagedPath).length()).isEqualTo(16L)

        val unknown = HostQueueItem("entry-2", "gone", "Gone", byteLength = null, durationMs = null)
        val failure = runCatching { staging.resolve(listOf(unknown), emptyList()) }.exceptionOrNull()
        assertThat(failure).hasMessageThat().contains("source permission is unavailable")
    }

    @Test
    fun discardReleaseAllAndOrphanCleanupDeleteTheCopies() = runTest {
        val staging = staging()
        val a = staging.stage(song("a"))
        val b = staging.stage(song("b"))

        staging.discard(listOf("a"))
        assertThat(a.exists()).isFalse()
        assertThat(staging.stagedSourceIds).containsExactly("b")

        staging.releaseAll()
        assertThat(b.exists()).isFalse()
        assertThat(staging.stagedSourceIds).isEmpty()

        val orphan = File(dir, "silent-disco-source-left-over.wav").apply { writeText("x") }
        val unrelated = File(dir, "other.tmp").apply { writeText("x") }
        staging.deleteOrphans()
        assertThat(orphan.exists()).isFalse()
        assertThat(unrelated.exists()).isTrue()
    }

    @Test
    fun aSourceLargerThanTheLimitIsRejectedBeforeCopying() {
        val staging = staging()
        val huge = song("huge").copy(sizeBytes = 9L * 1024 * 1024 * 1024)
        val failure = assertThrows(AudioFileAccessException::class.java) {
            kotlinx.coroutines.runBlocking { staging.stage(huge) }
        }
        assertThat(failure).hasMessageThat().contains("8 GiB")
        assertThat(validated).isEmpty()
    }
}
